//! Source-bound, reproducible image resources. Images survive missing source media.
use super::{asset_file, now, require_asset, unsigned};
use crate::desktop_core::transcription::Source;
use anyhow::{bail, Context, Result};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::path::Path;
use uuid::Uuid;

pub const MAX_FRAME_BYTES: usize = 8 * 1024 * 1024;
pub const MAX_FRAME_EDGE: u32 = 1280;

/// All times use source presentation PTS, with a zero at media time zero.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct KeyframeData {
    pub id: String,
    pub asset_id: String,
    pub file_id: String,
    pub clip_id: Option<String>,
    pub batch_id: String,
    pub sample_start_ms: u64,
    pub sample_end_ms: Option<u64>,
    pub sample_index: u32,
    pub sample_count: u32,
    pub requested_ms: u64,
    pub pts: u64,
    pub time_base_num: u32,
    pub time_base_den: u32,
    pub width: u32,
    pub height: u32,
    pub black: bool,
    pub created_at: u64,
}
impl KeyframeData {
    fn actual_ms(&self) -> u64 {
        ((u128::from(self.pts) * u128::from(self.time_base_num) * 1000
            + u128::from(self.time_base_den) / 2)
            / u128::from(self.time_base_den)) as u64
    }
    pub fn validate(&self) -> Result<()> {
        if self.id.is_empty()
            || self.batch_id.is_empty()
            || self.time_base_num == 0
            || self.time_base_den == 0
            || !(1..=9).contains(&self.sample_count)
            || self.sample_index >= self.sample_count
            || self.requested_ms > 9_007_199_254_740_991
            || self.sample_start_ms > self.requested_ms
            || self.width == 0
            || self.height == 0
            || self.width > MAX_FRAME_EDGE
            || self.height > MAX_FRAME_EDGE
        {
            bail!("KEYFRAME_INVALID_RESULT");
        }
        let actual = u128::from(self.pts) * u128::from(self.time_base_num) * 1000;
        let base = u128::from(self.time_base_den);
        if actual < u128::from(self.requested_ms) * base
            || actual > u128::from(9_007_199_254_740_991u64) * base
            || self.sample_end_ms.is_some_and(|end| {
                end <= self.sample_start_ms
                    || self.requested_ms >= end
                    || actual >= u128::from(end) * base
            })
        {
            bail!("KEYFRAME_RANGE_UNAVAILABLE");
        }
        if self.clip_id.is_none()
            && (self.sample_count != 1
                || self.sample_index != 0
                || self.sample_start_ms != self.requested_ms
                || self.sample_end_ms.is_some())
        {
            bail!("KEYFRAME_INVALID_RESULT");
        }
        Ok(())
    }
}
#[derive(Debug, Serialize)]
pub struct Keyframe {
    #[serde(flatten)]
    pub data: KeyframeData,
    pub actual_ms: u64,
    pub bytes: u64,
    pub missing: bool,
    pub source_changed: bool,
}
pub(crate) struct FrameRecord {
    pub data: KeyframeData,
    pub source: Source,
    pub body: Vec<u8>,
}

pub fn keyframes(db: &Connection, asset: &str) -> Result<Vec<Keyframe>> {
    require_asset(db, asset)?;
    let mut query = db.prepare("SELECT metadata,source_json,length(body) FROM asset_keyframes WHERE asset_id=?1 ORDER BY rowid DESC")?;
    let rows = query
        .query_map([asset], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                unsigned(r, 2)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    rows.into_iter()
        .map(|(metadata, source, bytes)| {
            let data: KeyframeData = serde_json::from_str(&metadata)?;
            data.validate()?;
            let source: Source = serde_json::from_str(&source)?;
            let file = asset_file(db, asset, &data.file_id)?;
            let current = Source::read(&file.path).ok();
            Ok(Keyframe {
                actual_ms: data.actual_ms(),
                data,
                bytes,
                missing: current.is_none(),
                source_changed: current.as_ref() != Some(&source),
            })
        })
        .collect()
}
pub(super) fn frame_records(db: &Connection) -> Result<Vec<FrameRecord>> {
    let mut query =
        db.prepare("SELECT metadata,source_json,body FROM asset_keyframes ORDER BY rowid")?;
    let rows = query
        .query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, Vec<u8>>(2)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    rows.into_iter()
        .map(|(data, source, body)| {
            Ok(FrameRecord {
                data: serde_json::from_str(&data)?,
                source: serde_json::from_str(&source)?,
                body,
            })
        })
        .collect()
}
pub fn keyframe_image(db: &Connection, asset: &str, id: &str) -> Result<Vec<u8>> {
    require_asset(db, asset)?;
    db.query_row(
        "SELECT body FROM asset_keyframes WHERE asset_id=?1 AND id=?2",
        params![asset, id],
        |r| r.get(0),
    )
    .optional()?
    .context("KEYFRAME_NOT_FOUND")
}
pub fn delete_keyframe(db: &Connection, asset: &str, id: &str) -> Result<()> {
    require_asset(db, asset)?;
    if db.execute(
        "DELETE FROM asset_keyframes WHERE asset_id=?1 AND id=?2",
        params![asset, id],
    )? != 1
    {
        bail!("KEYFRAME_NOT_FOUND");
    }
    Ok(())
}
pub fn validate_frame_image(body: &[u8], width: u32, height: u32) -> Result<()> {
    if body.len() < 33
        || body.len() > MAX_FRAME_BYTES
        || !body.starts_with(b"\x89PNG\r\n\x1a\n")
        || body.get(12..16) != Some(b"IHDR")
        || u32::from_be_bytes(body[16..20].try_into()?) != width
        || u32::from_be_bytes(body[20..24].try_into()?) != height
        || width == 0
        || height == 0
        || width > MAX_FRAME_EDGE
        || height > MAX_FRAME_EDGE
    {
        bail!("KEYFRAME_INVALID_RESULT");
    }
    let mut decoder = png::Decoder::new(std::io::Cursor::new(body));
    decoder.set_limits(png::Limits {
        bytes: 32 * 1024 * 1024,
    });
    let mut reader = decoder.read_info().context("KEYFRAME_INVALID_RESULT")?;
    if reader.info().width != width
        || reader.info().height != height
        || reader.info().animation_control.is_some()
    {
        bail!("KEYFRAME_INVALID_RESULT");
    }
    let size = reader
        .output_buffer_size()
        .filter(|size| *size <= 16 * 1024 * 1024)
        .context("KEYFRAME_INVALID_RESULT")?;
    reader
        .next_frame(&mut vec![0; size])
        .context("KEYFRAME_INVALID_RESULT")?;
    reader.finish().context("KEYFRAME_INVALID_RESULT")?;
    Ok(())
}
pub(super) fn validate_frame_reference(
    db: &Connection,
    data: &KeyframeData,
    source: &Source,
) -> Result<()> {
    data.validate()?;
    let file = asset_file(db, &data.asset_id, &data.file_id)?;
    if file.asset_id != data.asset_id {
        bail!("FILE_NOT_FOUND");
    }
    if let Some(clip_id) = &data.clip_id {
        let (file_id,start,end,saved): (String,u64,u64,String) = db.query_row("SELECT file_id,start_ms,end_ms,source_json FROM asset_clips WHERE asset_id=?1 AND id=?2",params![data.asset_id,clip_id],|r|Ok((r.get(0)?,unsigned(r,1)?,unsigned(r,2)?,r.get(3)?)))?;
        if file_id != data.file_id
            || data.sample_start_ms != start
            || data.sample_end_ms != Some(end)
            || serde_json::from_str::<Source>(&saved)? != *source
            || sample_points(start, end, data.sample_count)?.get(data.sample_index as usize)
                != Some(&data.requested_ms)
        {
            bail!("CLIP_SOURCE_MISMATCH");
        }
    }
    Ok(())
}
pub(super) fn insert_frame(
    db: &Connection,
    data: &KeyframeData,
    source: &Source,
    body: &[u8],
) -> Result<()> {
    validate_frame_reference(db, data, source)?;
    validate_frame_image(body, data.width, data.height)?;
    db.execute(
        "INSERT INTO asset_keyframes VALUES(?1,?2,?3,?4,?5,?6,?7)",
        params![
            data.id,
            data.asset_id,
            data.file_id,
            data.clip_id,
            serde_json::to_string(data)?,
            serde_json::to_string(source)?,
            body
        ],
    )?;
    Ok(())
}
pub fn sample_points(start: u64, end: u64, count: u32) -> Result<Vec<u64>> {
    if start >= end
        || !(1..=9).contains(&count)
        || end > 9_007_199_254_740_991
        || end - start < u64::from(count)
    {
        bail!("KEYFRAME_RANGE_INVALID");
    }
    Ok((0..count)
        .map(|i| {
            start
                + ((u128::from(end - start) * u128::from(2 * i + 1)) / u128::from(2 * count)) as u64
        })
        .collect())
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct FrameArtifact {
    filename: String,
    requested_ms: u64,
    pts: u64,
    time_base_num: u32,
    time_base_den: u32,
    width: u32,
    height: u32,
    black: bool,
    bytes: u64,
}
pub(crate) struct FrameBatch {
    pub asset_id: String,
    pub file_id: String,
    pub clip_id: Option<String>,
    pub id: String,
    pub source: Source,
    pub start_ms: u64,
    pub end_ms: Option<u64>,
    pub points: Vec<u64>,
}
impl FrameBatch {
    pub fn read(&self, stage: &Path, result: &serde_json::Value) -> Result<Vec<FrameRecord>> {
        let artifacts: Vec<FrameArtifact> =
            serde_json::from_value(result["frames"].clone()).context("KEYFRAME_INVALID_RESULT")?;
        if artifacts.len() != self.points.len() || self.points.is_empty() || self.points.len() > 9 {
            bail!("KEYFRAME_INVALID_RESULT");
        }
        let mut records = Vec::new();
        for (i, artifact) in artifacts.into_iter().enumerate() {
            if artifact.filename != format!("frame-{i:03}.png")
                || artifact.requested_ms != self.points[i]
                || artifact.bytes > MAX_FRAME_BYTES as u64
            {
                bail!("KEYFRAME_INVALID_RESULT");
            }
            let body = crate::files::read_cover(&stage.join(&artifact.filename))?;
            if body.len() as u64 != artifact.bytes {
                bail!("KEYFRAME_INVALID_RESULT");
            }
            let data = KeyframeData {
                id: Uuid::new_v4().to_string(),
                asset_id: self.asset_id.clone(),
                file_id: self.file_id.clone(),
                clip_id: self.clip_id.clone(),
                batch_id: self.id.clone(),
                sample_start_ms: self.start_ms,
                sample_end_ms: self.end_ms,
                sample_index: i as u32,
                sample_count: self.points.len() as u32,
                requested_ms: artifact.requested_ms,
                pts: artifact.pts,
                time_base_num: artifact.time_base_num,
                time_base_den: artifact.time_base_den,
                width: artifact.width,
                height: artifact.height,
                black: artifact.black,
                created_at: now(),
            };
            records.push(FrameRecord {
                data,
                source: self.source.clone(),
                body,
            });
        }
        Ok(records)
    }
}
pub(crate) fn store_frames(db: &Connection, records: &[FrameRecord]) -> Result<Vec<String>> {
    let tx = db.unchecked_transaction()?;
    for frame in records {
        insert_frame(&tx, &frame.data, &frame.source, &frame.body)?;
    }
    tx.commit()?;
    Ok(records.iter().map(|frame| frame.data.id.clone()).collect())
}
