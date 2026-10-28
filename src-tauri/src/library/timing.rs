use super::{sql_integer, unsigned};
use anyhow::{bail, Context, Result};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::{fs, path::Path};

const MAX_BYTES: u64 = 16 * 1024 * 1024;
const VAD: &str = "6840bae4c5c92ee8c04faaf4db23dd0105098d7f";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TranscriptSegment {
    pub start_ms: u64,
    pub end_ms: u64,
    pub text: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TranscriptTiming {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub operation_id: Option<String>,
    pub schema_version: u32,
    pub time_base: String,
    pub timing_source: String,
    pub precision: String,
    pub sample_rate: u32,
    pub frames: u64,
    pub vad_revision: String,
    pub segments: Vec<TranscriptSegment>,
}

impl TranscriptTiming {
    pub(crate) fn validate(&self, text: &str) -> Result<()> {
        if self.schema_version != 1
            || self.time_base != "media_zero"
            || self.timing_source != "fsmn_vad"
            || self.precision != "vad_segment"
            || self.sample_rate != 16000
            || self.frames == 0
            || self.frames > i64::MAX as u64
            || self.vad_revision != VAD
            || self.segments.len() > 100000
            || serde_json::to_vec(self)?.len() as u64 > MAX_BYTES
        {
            bail!("INVALID_ENGINE_RESULT");
        }
        let mut previous = 0;
        for row in &self.segments {
            if row.start_ms < previous
                || row.end_ms <= row.start_ms
                || row.end_ms > self.frames / 16
                || row.text.trim().is_empty()
                || row.text == "/sil"
            {
                bail!("INVALID_ENGINE_RESULT");
            }
            previous = row.end_ms;
        }
        if self
            .segments
            .iter()
            .map(|s| s.text.as_str())
            .collect::<Vec<_>>()
            .join("\n")
            != text
        {
            bail!("INVALID_ENGINE_RESULT");
        }
        Ok(())
    }

    pub(crate) fn artifact(
        stage: &Path,
        descriptor: &serde_json::Value,
        text: &str,
    ) -> Result<Self> {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Artifact {
            filename: String,
            bytes: u64,
        }
        let artifact: Artifact =
            serde_json::from_value(descriptor.clone()).context("INVALID_ENGINE_RESULT")?;
        if artifact.filename != "transcript-segments.json" || artifact.bytes > MAX_BYTES {
            bail!("INVALID_ENGINE_RESULT");
        }
        let path = stage.join("transcript-segments.json");
        let meta = fs::symlink_metadata(&path).context("INVALID_ENGINE_RESULT")?;
        if !meta.is_file() || meta.file_type().is_symlink() || meta.len() != artifact.bytes {
            bail!("INVALID_ENGINE_RESULT");
        }
        let value: Self = serde_json::from_slice(&fs::read(path).context("INVALID_ENGINE_RESULT")?)
            .context("INVALID_ENGINE_RESULT")?;
        value.validate(text)?;
        Ok(value)
    }
}

pub(super) fn store(db: &Connection, document: &str, timing: &TranscriptTiming) -> Result<()> {
    let operation = timing
        .operation_id
        .as_deref()
        .filter(|s| !s.is_empty() && s.len() <= 128)
        .context("INVALID_ENGINE_RESULT")?;
    db.execute(
        "INSERT INTO transcript_timings VALUES(?1,?2,?3,?4)",
        params![
            document,
            sql_integer(timing.frames)?,
            timing.vad_revision,
            operation
        ],
    )?;
    let mut insert = db.prepare("INSERT INTO transcript_segments VALUES(?1,?2,?3,?4,?5)")?;
    for (position, row) in timing.segments.iter().enumerate() {
        insert.execute(params![
            document,
            position as i64,
            sql_integer(row.start_ms)?,
            sql_integer(row.end_ms)?,
            row.text
        ])?;
    }
    Ok(())
}

pub(super) fn read(db: &Connection, document: &str) -> Result<Option<TranscriptTiming>> {
    let metadata = db
        .query_row(
            "SELECT frames,vad_revision,operation_id FROM transcript_timings WHERE document_id=?1",
            [document],
            |r| {
                Ok((
                    unsigned(r, 0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                ))
            },
        )
        .optional()?;
    let Some((frames, vad_revision, operation_id)) = metadata else {
        return Ok(None);
    };
    let mut query = db.prepare("SELECT start_ms,end_ms,text FROM transcript_segments WHERE document_id=?1 ORDER BY position")?;
    let segments = query
        .query_map([document], |r| {
            Ok(TranscriptSegment {
                start_ms: unsigned(r, 0)?,
                end_ms: unsigned(r, 1)?,
                text: r.get(2)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(Some(TranscriptTiming {
        operation_id: Some(operation_id),
        schema_version: 1,
        time_base: "media_zero".into(),
        timing_source: "fsmn_vad".into(),
        precision: "vad_segment".into(),
        sample_rate: 16000,
        frames,
        vad_revision,
        segments,
    }))
}
