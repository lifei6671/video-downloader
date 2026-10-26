use anyhow::{bail, Context, Result};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

pub const DEFAULT_TEMPLATE: &str = "{platform}/{user_name}-{content}";
const VARIABLES: [&str; 6] = [
    "platform",
    "user_name",
    "link_id",
    "content",
    "current_time",
    "quality",
];

pub fn validate(template: &str) -> Result<()> {
    if template.is_empty() || template.len() > 512 || template.contains('\\') {
        bail!("NAMING_INVALID");
    }
    let parts: Vec<_> = template.split('/').collect();
    if parts.len() > 4 {
        bail!("NAMING_INVALID");
    }
    for part in parts {
        if part.trim().is_empty() || matches!(part.trim(), "." | "..") {
            bail!("NAMING_INVALID");
        }
    }
    Ok(())
}

#[derive(Clone, Debug, serde::Serialize)]
pub struct PreviewSegment {
    pub text: String,
    pub variable: Option<&'static str>,
}

#[derive(serde::Serialize)]
pub struct FilenamePreview {
    pub path: String,
    pub segments: Vec<PreviewSegment>,
}

fn push_segment(segments: &mut Vec<PreviewSegment>, text: &str, variable: Option<&'static str>) {
    if text.is_empty() {
        return;
    }
    if let Some(last) = segments.last_mut().filter(|last| last.variable == variable) {
        last.text.push_str(text);
    } else {
        segments.push(PreviewSegment {
            text: text.into(),
            variable,
        });
    }
}

// Keep each character's source while applying the same filename cleanup used
// for downloads, so preview colors remain correct after trimming/truncation.
fn clean_segments(segments: Vec<PreviewSegment>, limit: usize) -> Vec<PreviewSegment> {
    let mut chars = Vec::new();
    let mut bytes = 0;
    'segments: for segment in segments {
        for ch in segment.text.chars() {
            let ch = if ch.is_control() || "<>:\"/\\|?*".contains(ch) {
                '_'
            } else {
                ch
            };
            if bytes + ch.len_utf8() > limit {
                break 'segments;
            }
            bytes += ch.len_utf8();
            chars.push((ch, segment.variable));
        }
    }
    let start = chars
        .iter()
        .position(|(ch, _)| !ch.is_whitespace())
        .unwrap_or(chars.len());
    let mut end = chars
        .iter()
        .rposition(|(ch, _)| !ch.is_whitespace())
        .map(|i| i + 1)
        .unwrap_or(start);
    while end > start && matches!(chars[end - 1].0, '.' | ' ') {
        end -= 1;
    }
    if start == end {
        return vec![PreviewSegment {
            text: "_".into(),
            variable: None,
        }];
    }
    let output: String = chars[start..end].iter().map(|(ch, _)| ch).collect();
    let stem = output
        .split('.')
        .next()
        .unwrap_or_default()
        .to_ascii_uppercase();
    if matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || ["COM", "LPT"].iter().any(|prefix| {
            stem.strip_prefix(prefix).is_some_and(|suffix| {
                matches!(
                    suffix,
                    "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" | "¹" | "²" | "³"
                )
            })
        })
    {
        chars.insert(start, ('_', None));
        end += 1;
    }
    let mut output = Vec::new();
    for (ch, variable) in &chars[start..end] {
        push_segment(&mut output, &ch.to_string(), *variable);
    }
    output
}

fn clean(text: &str, limit: usize) -> String {
    clean_segments(
        vec![PreviewSegment {
            text: text.into(),
            variable: None,
        }],
        limit,
    )
    .into_iter()
    .map(|segment| segment.text)
    .collect()
}

fn render_segments(template: &str, video: &Value, time: &str) -> Result<Vec<PreviewSegment>> {
    validate(template)?;
    let text = |key: &str| video[key].as_str().filter(|v| !v.trim().is_empty());
    let quality = match (video["width"].as_u64(), video["height"].as_u64()) {
        (Some(w), Some(h)) if w > 0 && h > 0 => format!("{}P", w.min(h)),
        _ if video["kind"] == "image_album" => "图集".into(),
        _ => "未知清晰度".into(),
    };
    let values = [
        text("platform").unwrap_or("unknown"),
        video["author_profile"]["nickname"]
            .as_str()
            .filter(|s| !s.trim().is_empty())
            .or_else(|| text("author"))
            .unwrap_or("未知作者"),
        text("video_id").unwrap_or("unknown"),
        text("title")
            .or_else(|| text("video_id"))
            .unwrap_or("未命名"),
        time,
        &quality,
    ];
    let parts: Vec<_> = template.split('/').collect();
    // Keep the full relative path bounded, including multibyte titles.
    let limit = 180 / parts.len();
    let mut output = Vec::new();
    for part in parts {
        let mut rest = part;
        let mut rendered = Vec::new();
        while let Some(start) = rest.find('{') {
            push_segment(&mut rendered, &rest[..start], None);
            rest = &rest[start..];
            if let Some(index) = VARIABLES.iter().position(|variable| {
                rest[1..]
                    .strip_prefix(*variable)
                    .is_some_and(|suffix| suffix.starts_with('}'))
            }) {
                push_segment(
                    &mut rendered,
                    &clean(values[index], limit),
                    Some(VARIABLES[index]),
                );
                rest = &rest[VARIABLES[index].len() + 2..];
            } else {
                push_segment(&mut rendered, "{", None);
                rest = &rest[1..];
            }
        }
        push_segment(&mut rendered, rest, None);
        if !output.is_empty() {
            push_segment(&mut output, "/", None);
        }
        for segment in clean_segments(rendered, limit) {
            push_segment(&mut output, &segment.text, segment.variable);
        }
    }
    Ok(output)
}

pub fn render(template: &str, video: &Value, time: &str) -> Result<String> {
    Ok(render_segments(template, video, time)?
        .into_iter()
        .map(|segment| segment.text)
        .collect())
}

pub fn current_time() -> String {
    chrono::Local::now().format("%Y%m%d_%H%M%S").to_string()
}

pub fn preview(template: &str) -> Result<FilenamePreview> {
    let mut segments = render_segments(
        template,
        &json!({"platform":"youtube", "author":"creator", "video_id":"sample-id",
            "title":"sample-video", "width":1920, "height":1080}),
        &current_time(),
    )?;
    push_segment(&mut segments, ".mp4", None);
    Ok(FilenamePreview {
        path: segments
            .iter()
            .map(|segment| segment.text.as_str())
            .collect(),
        segments,
    })
}

// Finalization still performs an atomic create-if-absent move. This only chooses
// a readable suffix for existing names; a concurrent external collision fails safely.
pub fn destination(directory: &Path, relative: &str, extension: Option<&str>) -> Result<PathBuf> {
    let root = directory.canonicalize().context("IO_ERROR")?;
    let relative = Path::new(relative);
    let mut parent = root.clone();
    for component in relative.parent().context("NAMING_INVALID")?.components() {
        let std::path::Component::Normal(name) = component else {
            bail!("NAMING_INVALID");
        };
        parent.push(name);
        match std::fs::create_dir(&parent) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error).context("IO_ERROR"),
        }
        if !parent
            .canonicalize()
            .context("IO_ERROR")?
            .starts_with(&root)
        {
            bail!("NAMING_INVALID");
        }
    }
    let name = relative
        .file_name()
        .and_then(|v| v.to_str())
        .context("NAMING_INVALID")?;
    for index in 0..10000 {
        let suffix = if index == 0 {
            String::new()
        } else {
            format!(" ({index})")
        };
        let ext = extension.map(|ext| format!(".{ext}")).unwrap_or_default();
        let path = parent.join(format!("{name}{suffix}{ext}"));
        match std::fs::symlink_metadata(&path) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(path),
            Err(error) => return Err(error).context("IO_ERROR"),
            Ok(_) => {}
        }
    }
    bail!("IO_ERROR: too many duplicate filenames")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preview_segments_keep_variable_sources_and_exact_final_path() {
        let preview =
            preview("youtube-{platform}/{user_name}-{link_id}-{content}-{quality}-{current_time}")
                .unwrap();
        assert_eq!(
            preview
                .segments
                .iter()
                .map(|s| s.text.as_str())
                .collect::<String>(),
            preview.path
        );
        assert_eq!(preview.segments[0].text, "youtube-");
        assert_eq!(preview.segments[0].variable, None);
        let variables: Vec<_> = preview.segments.iter().filter_map(|s| s.variable).collect();
        assert_eq!(
            variables,
            [
                "platform",
                "user_name",
                "link_id",
                "content",
                "quality",
                "current_time"
            ]
        );
        assert!(preview.path.ends_with(".mp4"));
        assert_eq!(preview.segments.last().unwrap().variable, None);
        for title in [
            "文".repeat(100),
            " CON.txt ".into(),
            "a/b? ".into(),
            "...".into(),
        ] {
            let video = json!({"platform":"youtube", "title":title, "video_id":"id"});
            let segments = render_segments(" {platform} /{content}. ", &video, "now").unwrap();
            let path: String = segments.iter().map(|s| s.text.as_str()).collect();
            assert_eq!(
                path,
                render(" {platform} /{content}. ", &video, "now").unwrap()
            );
            assert!(segments.iter().any(|s| s.variable == Some("content")));
            assert!(path.len() <= 182);
        }
    }

    #[test]
    fn rules_reject_malformed_paths() {
        for rule in [
            "",
            "/a",
            "a/",
            "../a",
            "a/../b",
            "a//b",
            "a\\b",
            "a/b/c/d/e",
        ] {
            assert!(validate(rule).is_err(), "{rule}");
        }
        assert!(validate(&"x".repeat(513)).is_err());
    }

    #[test]
    fn only_complete_supported_variables_are_substituted() {
        let video = json!({"platform":"youtube", "author":"creator", "video_id":"sample-id",
            "title":"sample-video", "width":1920, "height":1080});
        for (rule, expected) in [
            ("{unknown}", "{unknown}"),
            ("{content", "{content"),
            ("content}", "content}"),
            ("{}-{{}}", "{}-{{}}"),
            ("{{content}}", "{sample-video}"),
            ("}{content}{", "}sample-video{"),
            ("{unknown-{content}", "{unknown-sample-video"),
            (
                "{QUALITY}-{quality_extra}-{quality}",
                "{QUALITY}-{quality_extra}-1080P",
            ),
            (
                "中文{user_name}-{link_id}-{quality",
                "中文creator-sample-id-{quality",
            ),
            (
                "{platform}/{user_name}-{link_id}-{quality",
                "youtube/creator-sample-id-{quality",
            ),
        ] {
            assert!(validate(rule).is_ok(), "{rule}");
            assert_eq!(render(rule, &video, "now").unwrap(), expected, "{rule}");
            let preview = preview(rule).unwrap();
            assert_eq!(preview.path, format!("{expected}.mp4"), "{rule}");
            assert_eq!(
                preview
                    .segments
                    .iter()
                    .map(|s| s.text.as_str())
                    .collect::<String>(),
                preview.path
            );
        }
        let preview = preview("{unknown}-{content}-{quality").unwrap();
        assert_eq!(
            preview
                .segments
                .iter()
                .filter_map(|s| s.variable)
                .collect::<Vec<_>>(),
            ["content"]
        );
        assert_eq!(preview.segments[0].text, "{unknown}-");
        assert_eq!(preview.segments[0].variable, None);
        assert_eq!(preview.segments.last().unwrap().text, "-{quality.mp4");
        assert_eq!(preview.segments.last().unwrap().variable, None);
    }

    #[test]
    fn metadata_cannot_create_paths_and_names_are_portable() {
        let video = json!({"platform":"youtube","author":"../../CON", "title":"a/b:{link_id}?", "video_id":"123", "width":1080,"height":1920});
        assert_eq!(
            render("{platform}/{user_name}-{content}-{quality}", &video, "now").unwrap(),
            "youtube/.._.._CON-a_b_{link_id}_-1080P"
        );
        assert_eq!(
            render("{content}", &json!({"title":"CON.txt"}), "now").unwrap(),
            "_CON.txt"
        );
        assert_eq!(
            render("{content}", &json!({"title":"..."}), "now").unwrap(),
            "_"
        );
        let long = render(
            "{platform}/{content}",
            &json!({"title":"文".repeat(100)}),
            "now",
        )
        .unwrap();
        assert!(long.len() <= 180);
    }

    #[test]
    fn nested_destinations_never_replace_existing_files_or_albums() {
        let root = tempfile::tempdir().unwrap();
        let first = destination(root.path(), "youtube/creator-video", Some("webm")).unwrap();
        std::fs::write(&first, "keep").unwrap();
        let second = destination(root.path(), "youtube/creator-video", Some("webm")).unwrap();
        assert!(second.ends_with("creator-video (1).webm"));
        let album = destination(root.path(), "douyin/album", None).unwrap();
        std::fs::create_dir(&album).unwrap();
        assert!(destination(root.path(), "douyin/album", None)
            .unwrap()
            .ends_with("album (1)"));
        assert_eq!(std::fs::read_to_string(first).unwrap(), "keep");
    }
}
