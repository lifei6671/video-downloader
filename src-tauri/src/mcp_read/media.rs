//! Read saved references and sampled PNGs; never extract or open arbitrary media.
use super::*;
use base64::{engine::general_purpose::STANDARD, Engine};
#[cfg(feature = "mcp")]
use rmcp::schemars;

#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "mcp", derive(rmcp::schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct AudioInput {
    pub asset_id: String,
    pub file_id: String,
    #[cfg_attr(feature = "mcp", schemars(transform = super::types::json_integer))]
    pub start_ms: u64,
    #[cfg_attr(feature = "mcp", schemars(transform = super::types::json_integer))]
    pub end_ms: u64,
}
#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "mcp", derive(rmcp::schemars::JsonSchema))]
pub struct AudioExcerpt {
    pub asset_id: String,
    pub file_id: String,
    #[cfg_attr(feature = "mcp", schemars(transform = super::types::json_integer))]
    pub start_ms: u64,
    #[cfg_attr(feature = "mcp", schemars(transform = super::types::json_integer))]
    pub end_ms: u64,
    #[cfg_attr(feature = "mcp", schemars(transform = super::types::json_integer))]
    pub actual_start_ms: u64,
    #[cfg_attr(feature = "mcp", schemars(transform = super::types::json_integer))]
    pub actual_end_ms: u64,
    pub format: String,
    #[cfg_attr(feature = "mcp", schemars(transform = super::types::json_integer))]
    pub sample_rate: u32,
    #[cfg_attr(feature = "mcp", schemars(transform = super::types::json_integer))]
    pub channels: u32,
    #[cfg_attr(feature = "mcp", schemars(transform = super::types::json_integer))]
    pub bits_per_sample: u32,
    #[cfg_attr(feature = "mcp", schemars(transform = super::types::json_integer))]
    pub frames: u64,
    pub source_changed: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "mcp", schemars(skip))]
    pub audio_base64: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "mcp", derive(rmcp::schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct ClipInput {
    pub asset_id: String,
    pub clip_id: Option<String>,
    pub cursor: Option<String>,
    #[serde(default = "super::types::page_limit")]
    #[cfg_attr(feature = "mcp", schemars(transform = super::types::json_integer))]
    pub limit: u32,
}
#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "mcp", derive(rmcp::schemars::JsonSchema))]
pub struct Clip {
    pub id: String,
    pub asset_id: String,
    pub file_id: String,
    pub title: String,
    pub reason: String,
    #[cfg_attr(feature = "mcp", schemars(transform = super::types::json_integer))]
    pub start_ms: u64,
    #[cfg_attr(feature = "mcp", schemars(transform = super::types::json_integer))]
    pub end_ms: u64,
    pub document_id: Option<String>,
    #[cfg_attr(feature = "mcp", schemars(transform = super::types::json_integer))]
    pub document_version: Option<u64>,
    pub quote_excerpt: Option<String>,
    pub quote_truncated: bool,
    pub missing: bool,
    pub source_changed: bool,
}
#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "mcp", derive(rmcp::schemars::JsonSchema))]
pub struct Clips {
    pub clips: Page<Clip>,
    pub scope: String,
}
pub(super) fn clips(db: &Connection, input: ClipInput) -> Result<Clips> {
    require_collected(db, &input.asset_id)?;
    if let Some(value) = &input.clip_id {
        id(value)?;
    }
    let all = library::clips(db, &input.asset_id)?;
    if input
        .clip_id
        .as_ref()
        .is_some_and(|id| !all.iter().any(|c| &c.id == id))
    {
        bail!("CLIP_NOT_FOUND");
    }
    let items = all
        .into_iter()
        .filter(|c| input.clip_id.as_ref().is_none_or(|id| &c.id == id))
        .map(|c| Clip {
            id: c.id,
            asset_id: c.asset_id,
            file_id: c.file_id,
            title: c.title,
            reason: c.reason,
            start_ms: c.start_ms,
            end_ms: c.end_ms,
            document_id: c.document_id,
            document_version: c.document_version,
            quote_excerpt: c.quote.as_ref().map(|q| q.chars().take(120).collect()),
            quote_truncated: c.quote.as_ref().is_some_and(|q| q.chars().count() > 120),
            missing: c.missing,
            source_changed: c.source_changed,
        })
        .collect();
    Ok(Clips {
        clips: page(items, input.cursor.as_deref(), input.limit, |c| &c.id)?,
        scope: "Saved clip references only; video/audio bytes are unsupported. Read the pinned document with read_transcript; get_keyframes reads saved stills. Quotes are excerpts, not word-aligned subtitles.".into(),
    })
}
fn frame_limit() -> u32 {
    3
}
fn images() -> bool {
    true
}
fn image_type() -> String {
    "image".into()
}
#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "mcp", derive(rmcp::schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct KeyframesInput {
    pub asset_id: String,
    pub file_id: Option<String>,
    pub clip_id: Option<String>,
    #[cfg_attr(feature = "mcp", schemars(transform = super::types::json_integer))]
    pub start_ms: Option<u64>,
    #[cfg_attr(feature = "mcp", schemars(transform = super::types::json_integer))]
    pub end_ms: Option<u64>,
    pub cursor: Option<String>,
    #[serde(default = "frame_limit")]
    #[cfg_attr(feature = "mcp", schemars(transform = super::types::json_integer))]
    pub limit: u32,
    #[serde(default = "images")]
    pub include_images: bool,
    #[serde(default = "image_type")]
    pub media_type: String,
}
#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "mcp", derive(rmcp::schemars::JsonSchema))]
pub struct Frame {
    pub id: String,
    pub asset_id: String,
    pub file_id: String,
    pub clip_id: Option<String>,
    pub batch_id: String,
    #[cfg_attr(feature = "mcp", schemars(transform = super::types::json_integer))]
    pub sample_start_ms: u64,
    #[cfg_attr(feature = "mcp", schemars(transform = super::types::json_integer))]
    pub sample_end_ms: Option<u64>,
    #[cfg_attr(feature = "mcp", schemars(transform = super::types::json_integer))]
    pub sample_index: u32,
    #[cfg_attr(feature = "mcp", schemars(transform = super::types::json_integer))]
    pub sample_count: u32,
    #[cfg_attr(feature = "mcp", schemars(transform = super::types::json_integer))]
    pub requested_ms: u64,
    #[cfg_attr(feature = "mcp", schemars(transform = super::types::json_integer))]
    pub actual_ms: u64,
    #[cfg_attr(feature = "mcp", schemars(transform = super::types::json_integer))]
    pub pts: u64,
    #[cfg_attr(feature = "mcp", schemars(transform = super::types::json_integer))]
    pub time_base_num: u32,
    #[cfg_attr(feature = "mcp", schemars(transform = super::types::json_integer))]
    pub time_base_den: u32,
    #[cfg_attr(feature = "mcp", schemars(transform = super::types::json_integer))]
    pub width: u32,
    #[cfg_attr(feature = "mcp", schemars(transform = super::types::json_integer))]
    pub height: u32,
    pub black: bool,
    pub missing: bool,
    pub source_changed: bool,
    pub image_status: String,
    pub image_is_preview: bool,
    #[cfg_attr(feature = "mcp", schemars(transform = super::types::json_integer))]
    pub image_width: Option<u32>,
    #[cfg_attr(feature = "mcp", schemars(transform = super::types::json_integer))]
    pub image_height: Option<u32>,
    #[cfg_attr(feature = "mcp", schemars(skip))]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub image_base64: Option<String>,
}
#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "mcp", derive(rmcp::schemars::JsonSchema))]
pub struct Keyframes {
    pub frames: Page<Frame>,
    pub scope: String,
}
pub(super) fn keyframes(db: &Connection, input: KeyframesInput) -> Result<Keyframes> {
    require_collected(db, &input.asset_id)?;
    if input.media_type != "image" {
        bail!("MEDIA_UNSUPPORTED");
    }
    if !(1..=9).contains(&input.limit)
        || input
            .start_ms
            .zip(input.end_ms)
            .is_some_and(|(s, e)| s >= e)
    {
        bail!("INVALID_INPUT");
    }
    if let Some(file) = &input.file_id {
        id(file)?;
        if !library::asset_detail(db, &input.asset_id)?
            .files
            .iter()
            .any(|f| &f.id == file)
        {
            bail!("FILE_NOT_FOUND");
        }
    }
    if let Some(clip) = &input.clip_id {
        id(clip)?;
        if !library::clips(db, &input.asset_id)?
            .iter()
            .any(|c| &c.id == clip)
        {
            bail!("CLIP_NOT_FOUND");
        }
    }
    let items = library::keyframes(db, &input.asset_id)?
        .into_iter()
        .filter(|f| {
            input
                .file_id
                .as_ref()
                .is_none_or(|id| &f.data.file_id == id)
                && input
                    .clip_id
                    .as_ref()
                    .is_none_or(|id| f.data.clip_id.as_ref() == Some(id))
                && input.start_ms.is_none_or(|s| {
                    u128::from(f.data.pts) * u128::from(f.data.time_base_num) * 1000
                        >= u128::from(s) * u128::from(f.data.time_base_den)
                })
                && input.end_ms.is_none_or(|e| {
                    u128::from(f.data.pts) * u128::from(f.data.time_base_num) * 1000
                        < u128::from(e) * u128::from(f.data.time_base_den)
                })
        })
        .collect();
    let selected = page(items, input.cursor.as_deref(), input.limit, |f| &f.data.id)?;
    // Respect the existing 1 MiB IPC frame, including JSON/base64 overhead.
    // Large source PNGs use an explicitly labelled, bounded preview. The saved image is unchanged.
    let mut budget = 512 * 1024;
    let mut frames = Vec::new();
    for f in selected.items {
        let mut image_status = "not_requested";
        let mut image_base64 = None;
        let mut image_is_preview = false;
        let mut image_width = None;
        let mut image_height = None;
        if input.include_images {
            let mut body = library::keyframe_image(db, &input.asset_id, &f.data.id)?;
            let (mut width, mut height) = (f.data.width, f.data.height);
            if body.len() > 512 * 1024 {
                let decoded = image::load_from_memory_with_format(&body, image::ImageFormat::Png)?;
                let preview = decoded.thumbnail(384, 384).to_rgb8();
                (width, height) = preview.dimensions();
                let mut encoded = std::io::Cursor::new(Vec::new());
                preview.write_to(&mut encoded, image::ImageFormat::Png)?;
                body = encoded.into_inner();
                image_is_preview = true;
            }
            if body.len() > 512 * 1024 {
                image_status = "too_large";
            } else if body.len() > budget {
                image_status = "page_budget_exceeded";
            } else {
                budget -= body.len();
                image_base64 = Some(STANDARD.encode(body));
                image_status = "included";
                image_width = Some(width);
                image_height = Some(height);
            }
        }
        let d = f.data;
        frames.push(Frame {
            id: d.id,
            asset_id: d.asset_id,
            file_id: d.file_id,
            clip_id: d.clip_id,
            batch_id: d.batch_id,
            sample_start_ms: d.sample_start_ms,
            sample_end_ms: d.sample_end_ms,
            sample_index: d.sample_index,
            sample_count: d.sample_count,
            requested_ms: d.requested_ms,
            actual_ms: f.actual_ms,
            pts: d.pts,
            time_base_num: d.time_base_num,
            time_base_den: d.time_base_den,
            width: d.width,
            height: d.height,
            black: d.black,
            missing: f.missing,
            source_changed: f.source_changed,
            image_status: image_status.into(),
            image_is_preview,
            image_width,
            image_height,
            image_base64,
        });
    }
    Ok(Keyframes { frames: Page {items: frames, next_cursor: selected.next_cursor, total: selected.total},
        scope: "Saved PNG samples only, ordered by stable ID. Not continuous video, transitions, motion or audio. Times and width/height describe the saved source; image_width/image_height describe the delivered PNG. image_is_preview=true marks a reduced preview (max edge 384): fine text/details may be unreadable, never claim original resolution. Saved PNGs and PTS are unchanged. Missing/changed source flags remain historical. Video/audio return MEDIA_UNSUPPORTED. Retry page_budget_exceeded with limit=1 and the preceding cursor. Tools work without Prompts.".into() })
}

#[cfg(test)]
#[path = "media_tests.rs"]
mod tests;
