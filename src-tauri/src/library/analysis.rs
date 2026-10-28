//! Structured learning notes. Media remains owned by immutable clips/keyframes.
use super::{documents::validate_text, Citation, NewNote, Note, NoteOrigin};
use anyhow::{bail, Context, Result};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ObservationKind {
    Visual,
    Captions,
    Transition,
    Sound,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ObservationEvidence {
    Video,
    Audio,
    Keyframe,
    Unverified,
}
impl ObservationEvidence {
    fn label(&self) -> &'static str {
        match self {
            Self::Video => "观看视频",
            Self::Audio => "聆听声音",
            Self::Keyframe => "仅看关键帧，未核验连续运动",
            Self::Unverified => "待核验",
        }
    }
}
impl ObservationKind {
    fn label(&self) -> &'static str {
        match self {
            Self::Visual => "画面",
            Self::Captions => "字幕",
            Self::Transition => "转场",
            Self::Sound => "声音",
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ObservationInput {
    pub dimension: ObservationKind,
    pub evidence: ObservationEvidence,
    pub text: String,
    pub clip_id: String,
    pub keyframe_id: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum AnalysisInput {
    Copywriting {
        opening: String,
        viewpoint: String,
        evidence: String,
        turn: String,
        ending: String,
        takeaways: String,
    },
    Technique {
        observations: Vec<ObservationInput>,
    },
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Observation {
    pub dimension: ObservationKind,
    pub evidence: ObservationEvidence,
    pub text: String,
    pub clip_id: String,
    pub file_id: String,
    pub clip_title: String,
    pub start_ms: u64,
    pub end_ms: u64,
    pub keyframe_id: Option<String>,
    pub frame_actual_ms: Option<u64>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum AnalysisContent {
    Copywriting {
        opening: String,
        viewpoint: String,
        evidence: String,
        turn: String,
        ending: String,
        takeaways: String,
    },
    Technique {
        observations: Vec<Observation>,
    },
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NoteAnalysis {
    pub content: AnalysisContent,
    pub human_revised: bool,
}
fn field(value: &str) -> Result<()> {
    if value.len() > 64 * 1024 {
        bail!("ANALYSIS_INVALID");
    }
    Ok(())
}
fn timestamp(ms: u64) -> String {
    format!("{:02}:{:02}.{:03}", ms / 60000, ms / 1000 % 60, ms % 1000)
}
impl NoteAnalysis {
    pub fn note_kind(&self) -> &'static str {
        match self.content {
            AnalysisContent::Copywriting { .. } => "copywriting_analysis",
            AnalysisContent::Technique { .. } => "technique_analysis",
        }
    }
    pub fn body(&self) -> Result<String> {
        let result = match &self.content {
            AnalysisContent::Copywriting {
                opening,
                viewpoint,
                evidence,
                turn,
                ending,
                takeaways,
            } => {
                let fields = [
                    ("开头", opening),
                    ("观点", viewpoint),
                    ("论据", evidence),
                    ("转折", turn),
                    ("结尾", ending),
                    ("可借鉴点", takeaways),
                ];
                if fields.iter().all(|(_, text)| text.trim().is_empty()) {
                    bail!("ANALYSIS_INVALID");
                }
                let mut sections = Vec::new();
                for (label, text) in fields {
                    field(text)?;
                    sections.push(format!(
                        "## {label}\n{}",
                        if text.trim().is_empty() {
                            "未记录"
                        } else {
                            text
                        }
                    ));
                }
                sections.join("\n\n")
            }
            AnalysisContent::Technique { observations } => {
                if observations.is_empty() || observations.len() > 64 {
                    bail!("ANALYSIS_INVALID");
                }
                let mut sections = Vec::new();
                for row in observations {
                    if row.evidence == ObservationEvidence::Keyframe
                        && (row.dimension == ObservationKind::Sound || row.keyframe_id.is_none())
                    {
                        bail!("ANALYSIS_EVIDENCE_INVALID");
                    }
                    field(&row.text)?;
                    field(&row.clip_title)?;
                    if row.text.trim().is_empty()
                        || row.clip_id.is_empty()
                        || row.file_id.is_empty()
                        || row.start_ms >= row.end_ms
                        || row.end_ms > 9_007_199_254_740_991
                        || row.keyframe_id.as_ref().is_some_and(|s| s.is_empty())
                        || row.keyframe_id.is_some() != row.frame_actual_ms.is_some()
                        || row
                            .frame_actual_ms
                            .is_some_and(|ms| ms < row.start_ms || ms > row.end_ms)
                    {
                        bail!("ANALYSIS_INVALID");
                    }
                    sections.push(format!(
                        "## {} {}–{}\n片段：{}\n依据：{}\n{}{}",
                        row.dimension.label(),
                        timestamp(row.start_ms),
                        timestamp(row.end_ms),
                        row.clip_title,
                        row.evidence.label(),
                        row.text,
                        row.frame_actual_ms
                            .map(|ms| format!("\n关键帧：{}", timestamp(ms)))
                            .unwrap_or_default()
                    ));
                }
                sections.join("\n\n")
            }
        };
        validate_text(&result)?;
        Ok(result)
    }
}
pub(super) fn read(db: &Connection, id: &str) -> Result<Option<NoteAnalysis>> {
    db.query_row(
        "SELECT data FROM note_analyses WHERE note_id=?1",
        [id],
        |r| r.get::<_, String>(0),
    )
    .optional()?
    .map(|text| serde_json::from_str(&text).context("DATABASE_ERROR"))
    .transpose()
}
pub(super) fn store(db: &Connection, id: &str, value: &NoteAnalysis) -> Result<()> {
    db.execute("INSERT INTO note_analyses VALUES(?1,?2) ON CONFLICT(note_id) DO UPDATE SET data=excluded.data",params![id,serde_json::to_string(value)?])?;
    Ok(())
}
pub(super) fn validate_note(db: &Connection, note: &Note) -> Result<()> {
    let Some(value) = &note.analysis else {
        return Ok(());
    };
    if value.note_kind() != note.kind
        || value.body()? != note.body
        || matches!(note.origin, NoteOrigin::Human)
            && (note.model_name.is_some() || value.human_revised)
        || matches!(note.origin, NoteOrigin::Ai)
            && note
                .model_name
                .as_ref()
                .is_none_or(|s| s.trim().is_empty() || s.len() > 200)
    {
        bail!("ANALYSIS_INVALID");
    }
    if let AnalysisContent::Technique { observations } = &value.content {
        let clips = super::clips(db, &note.asset_id)?;
        let frames = super::keyframes(db, &note.asset_id)?;
        for row in observations {
            super::asset_file(db, &note.asset_id, &row.file_id)?;
            let owner: Option<String> = db
                .query_row(
                    "SELECT asset_id FROM asset_clips WHERE id=?1",
                    [&row.clip_id],
                    |r| r.get(0),
                )
                .optional()?;
            if owner.as_ref().is_some_and(|id| id != &note.asset_id) {
                bail!("ANALYSIS_REFERENCE_INVALID");
            }
            if let Some(clip) = clips.iter().find(|c| c.id == row.clip_id) {
                if clip.file_id != row.file_id
                    || clip.start_ms != row.start_ms
                    || clip.end_ms != row.end_ms
                    || clip.title != row.clip_title
                {
                    bail!("ANALYSIS_REFERENCE_INVALID");
                }
            }
            if let Some(id) = &row.keyframe_id {
                let owner: Option<String> = db
                    .query_row(
                        "SELECT asset_id FROM asset_keyframes WHERE id=?1",
                        [id],
                        |r| r.get(0),
                    )
                    .optional()?;
                if owner.as_ref().is_some_and(|id| id != &note.asset_id) {
                    bail!("ANALYSIS_REFERENCE_INVALID");
                }
                if let Some(frame) = frames.iter().find(|f| &f.data.id == id) {
                    if frame.data.file_id != row.file_id
                        || !frame_within(&frame.data, row.start_ms, row.end_ms)
                        || Some(frame.actual_ms) != row.frame_actual_ms
                    {
                        bail!("ANALYSIS_REFERENCE_INVALID");
                    }
                    if let Some(clip) = clips.iter().find(|c| c.id == row.clip_id) {
                        validate_frame_source(db, clip, id)?;
                    }
                }
            }
        }
    }
    Ok(())
}
pub(super) fn frame_within(frame: &super::KeyframeData, start: u64, end: u64) -> bool {
    let actual = u128::from(frame.pts) * u128::from(frame.time_base_num) * 1000;
    let base = u128::from(frame.time_base_den);
    base != 0 && actual >= u128::from(start) * base && actual < u128::from(end) * base
}

fn validate_frame_source(db: &Connection, clip: &super::Clip, frame: &str) -> Result<()> {
    let (a,b):(String,String)=db.query_row("SELECT c.source_json,k.source_json FROM asset_clips c,asset_keyframes k WHERE c.id=?1 AND k.id=?2",params![clip.id,frame],|r|Ok((r.get(0)?,r.get(1)?)))?;
    let a: crate::desktop_core::transcription::Source = serde_json::from_str(&a)?;
    let b = serde_json::from_str(&b)?;
    if a != b {
        bail!("ANALYSIS_REFERENCE_INVALID");
    }
    Ok(())
}
fn build(
    db: &Connection,
    asset: &str,
    input: &AnalysisInput,
    old: Option<&NoteAnalysis>,
) -> Result<NoteAnalysis> {
    let content = match input {
        AnalysisInput::Copywriting {
            opening,
            viewpoint,
            evidence,
            turn,
            ending,
            takeaways,
        } => AnalysisContent::Copywriting {
            opening: opening.clone(),
            viewpoint: viewpoint.clone(),
            evidence: evidence.clone(),
            turn: turn.clone(),
            ending: ending.clone(),
            takeaways: takeaways.clone(),
        },
        AnalysisInput::Technique { observations } => {
            if observations.len() > 64 {
                bail!("ANALYSIS_INVALID");
            }
            let clips = super::clips(db, asset)?;
            let frames = super::keyframes(db, asset)?;
            let mut rows = Vec::new();
            for input in observations {
                if let Some(clip) = clips.iter().find(|c| c.id == input.clip_id) {
                    let prior = old.and_then(|old| {
                        if let AnalysisContent::Technique { observations } = &old.content {
                            observations.iter().find(|row| {
                                row.clip_id == input.clip_id && row.keyframe_id == input.keyframe_id
                            })
                        } else {
                            None
                        }
                    });
                    let frame = input
                        .keyframe_id
                        .as_ref()
                        .and_then(|id| frames.iter().find(|f| &f.data.id == id));
                    if input.keyframe_id.is_some() && frame.is_none() && prior.is_none() {
                        bail!("KEYFRAME_NOT_FOUND");
                    }
                    if let Some(frame) = frame {
                        if frame.data.file_id != clip.file_id
                            || !frame_within(&frame.data, clip.start_ms, clip.end_ms)
                        {
                            bail!("ANALYSIS_REFERENCE_INVALID");
                        }
                        validate_frame_source(db, clip, &frame.data.id)?;
                    }
                    rows.push(Observation {
                        dimension: input.dimension.clone(),
                        evidence: input.evidence.clone(),
                        text: input.text.clone(),
                        clip_id: clip.id.clone(),
                        file_id: clip.file_id.clone(),
                        clip_title: clip.title.clone(),
                        start_ms: clip.start_ms,
                        end_ms: clip.end_ms,
                        keyframe_id: input.keyframe_id.clone(),
                        frame_actual_ms: frame
                            .map(|f| f.actual_ms)
                            .or_else(|| prior.and_then(|p| p.frame_actual_ms)),
                    });
                } else {
                    let saved = old
                        .and_then(|old| {
                            if let AnalysisContent::Technique { observations } = &old.content {
                                observations.iter().find(|row| {
                                    row.clip_id == input.clip_id
                                        && row.keyframe_id == input.keyframe_id
                                })
                            } else {
                                None
                            }
                        })
                        .context("CLIP_NOT_FOUND")?;
                    rows.push(Observation {
                        dimension: input.dimension.clone(),
                        evidence: input.evidence.clone(),
                        text: input.text.clone(),
                        ..saved.clone()
                    });
                }
            }
            AnalysisContent::Technique { observations: rows }
        }
    };
    let value = NoteAnalysis {
        content,
        human_revised: old.is_some_and(|v| v.human_revised),
    };
    value.body()?;
    Ok(value)
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NewAnalysis {
    pub content: AnalysisInput,
    pub origin: NoteOrigin,
    pub model_name: Option<String>,
    #[serde(default)]
    pub citations: Vec<Citation>,
}
pub fn create_analysis(db: &Connection, asset: &str, input: &NewAnalysis) -> Result<String> {
    match input.origin {
        NoteOrigin::Human if input.model_name.is_some() => bail!("ANALYSIS_INVALID"),
        NoteOrigin::Ai
            if input
                .model_name
                .as_ref()
                .is_none_or(|s| s.trim().is_empty() || s.len() > 200) =>
        {
            bail!("ANALYSIS_MODEL_REQUIRED")
        }
        _ => {}
    }
    let analysis = build(db, asset, &input.content, None)?;
    let body = analysis.body()?;
    let tx = db.unchecked_transaction()?;
    let id = super::create_note(
        &tx,
        asset,
        NewNote {
            kind: analysis.note_kind(),
            body: &body,
            origin: input.origin,
            model_name: input.model_name.as_deref(),
            citations: &input.citations,
        },
    )?;
    store(&tx, &id, &analysis)?;
    tx.commit()?;
    Ok(id)
}
pub fn edit_analysis(
    db: &Connection,
    asset: &str,
    id: &str,
    input: &AnalysisInput,
    revision: u64,
) -> Result<()> {
    let old = super::read_note(db, asset, id)?;
    let saved = old.analysis.as_ref().context("ANALYSIS_REQUIRED")?;
    let mut analysis = build(db, asset, input, Some(saved))?;
    if analysis.note_kind() != old.kind {
        bail!("ANALYSIS_INVALID");
    }
    analysis.human_revised = saved.human_revised || matches!(old.origin, NoteOrigin::Ai);
    let body = analysis.body()?;
    let tx = db.unchecked_transaction()?;
    if tx.execute("UPDATE asset_notes SET body=?1,revision=revision+1,updated_at=?2 WHERE id=?3 AND asset_id=?4 AND revision=?5",params![body,super::sql_integer(super::now())?,id,asset,super::sql_integer(revision)?])?!=1 { bail!("NOTE_CONFLICT"); }
    store(&tx, id, &analysis)?;
    tx.commit()?;
    Ok(())
}
