use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::Read,
    path::{Path, PathBuf},
};

#[derive(Deserialize)]
pub struct Video {
    pub platform: String,
    pub video_id: String,
    pub canonical_url: String,
    #[serde(default = "video_kind")]
    pub kind: String,
    #[serde(default = "default_resolver")]
    pub resolver: String,
}

fn video_kind() -> String {
    "video".into()
}
fn default_resolver() -> String {
    "yt-dlp".into()
}

impl Video {
    pub fn validate(&self) -> Result<()> {
        if !matches!(self.kind.as_str(), "video" | "image_album")
            || !matches!(self.resolver.as_str(), "yt-dlp" | "dtk")
            || (self.kind == "image_album" && self.resolver != "dtk")
            || !matches!(self.platform.as_str(), "douyin" | "tiktok")
            || self.video_id.is_empty()
            || self.video_id.len() > 30
            || !self.video_id.bytes().all(|c| c.is_ascii_digit())
        {
            bail!("INVALID_ENGINE_RESULT");
        }
        Ok(())
    }
}

#[derive(Serialize)]
pub struct Downloaded {
    pub path: PathBuf,
    pub bytes: u64,
}

pub fn finalize(
    video: &Video,
    result: &Value,
    staging: &Path,
    directory: &Path,
) -> Result<Downloaded> {
    if video.kind == "image_album" {
        return finalize_album(
            video,
            result,
            staging,
            &directory.join(format!("{}_{}", video.platform, video.video_id)),
            |_, _, _| Ok(()),
        );
    }
    let container = result["container"]
        .as_str()
        .context("INVALID_ENGINE_RESULT")?;
    let path = directory.join(format!(
        "{}_{}.{}",
        video.platform, video.video_id, container
    ));
    finalize_to(video, result, staging, &path, |_, _| Ok(()))
}

pub fn finalize_to(
    video: &Video,
    result: &Value,
    staging: &Path,
    path: &Path,
    before_move: impl FnOnce(&Path, u64) -> Result<()>,
) -> Result<Downloaded> {
    video.validate()?;
    if result["platform"] != video.platform || result["video_id"] != video.video_id {
        bail!("INVALID_ENGINE_RESULT");
    }
    let container = result["container"]
        .as_str()
        .context("INVALID_ENGINE_RESULT")?;
    if !matches!(container, "mp4" | "webm") {
        bail!("INVALID_MEDIA_CONTENT");
    }
    let filename = format!("video.{container}");
    if result["filename"] != filename {
        bail!("INVALID_ENGINE_RESULT");
    }
    let source = staging.join(filename);
    let metadata = fs::symlink_metadata(&source).context("IO_ERROR")?;
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        bail!("INVALID_ENGINE_RESULT");
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if metadata.file_attributes() & 0x400 != 0 {
            bail!("INVALID_ENGINE_RESULT");
        }
    }
    let size = metadata.len();
    if size == 0 || result["bytes"].as_u64() != Some(size) {
        bail!("INCOMPLETE_MEDIA");
    }
    let mut file = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(&source)
        .context("IO_ERROR")?;
    let mut head = [0; 12];
    file.read_exact(&mut head)
        .context("INVALID_MEDIA_CONTENT")?;
    let valid = match container {
        "mp4" => &head[4..8] == b"ftyp",
        "webm" => head.starts_with(&[0x1a, 0x45, 0xdf, 0xa3]),
        _ => false,
    };
    if !valid {
        bail!("INVALID_MEDIA_CONTENT");
    }
    file.sync_all().context("IO_ERROR")?;
    drop(file);
    before_move(&source, size)?;
    // Platform-specific create-if-absent move on the same volume (also works on
    // Windows filesystems without hard links). Never check then overwrite.
    tempfile::TempPath::try_from_path(source)
        .context("IO_ERROR")?
        .persist_noclobber(path)
        .map_err(|_| anyhow::anyhow!("IO_ERROR: destination exists or cannot finalize"))?;
    Ok(Downloaded {
        path: path.to_owned(),
        bytes: size,
    })
}

pub fn read_cover(path: &Path) -> Result<Vec<u8>> {
    let metadata = fs::symlink_metadata(path).context("IO_ERROR")?;
    if !metadata.is_file() || metadata.file_type().is_symlink() || metadata.len() > 10 * 1024 * 1024
    {
        bail!("INVALID_MEDIA_CONTENT");
    }
    let bytes = fs::read(path).context("IO_ERROR")?;
    if !(bytes.starts_with(&[0xff, 0xd8, 0xff])
        || bytes.starts_with(b"\x89PNG\r\n\x1a\n")
        || (bytes.starts_with(b"RIFF") && bytes.get(8..12) == Some(b"WEBP")))
    {
        bail!("INVALID_MEDIA_CONTENT");
    }
    Ok(bytes)
}

pub fn finalize_cover(result: &Value, staging: &Path, target: &Path) -> Result<PathBuf> {
    if result["cover_filename"] != "cover" {
        bail!("INVALID_ENGINE_RESULT");
    }
    let source = staging.join("cover");
    let bytes = read_cover(&source)?;
    if result["cover_bytes"].as_u64() != Some(bytes.len() as u64) {
        bail!("INCOMPLETE_MEDIA");
    }
    let extension = if bytes.starts_with(&[0xff, 0xd8, 0xff]) {
        "jpg"
    } else if bytes.starts_with(b"\x89PNG") {
        "png"
    } else {
        "webp"
    };
    let target = target.with_extension(format!("cover.{extension}"));
    fs::OpenOptions::new()
        .write(true)
        .open(&source)?
        .sync_all()?;
    tempfile::TempPath::try_from_path(source)?
        .persist_noclobber(&target)
        .map_err(|_| anyhow::anyhow!("IO_ERROR"))?;
    Ok(target.to_owned())
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AlbumFile {
    pub filename: String,
    pub bytes: u64,
    pub sha256: String,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub quality_source: String,
}

fn image_extension(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(&[0xff, 0xd8, 0xff]) {
        Some("jpeg")
    } else if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some("png")
    } else if bytes.starts_with(b"RIFF") && bytes.get(8..12) == Some(b"WEBP") {
        Some("webp")
    } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        Some("gif")
    } else if bytes.get(4..8) == Some(b"ftyp")
        && matches!(bytes.get(8..12), Some(b"avif" | b"avis"))
    {
        Some("avif")
    } else if bytes.get(4..8) == Some(b"ftyp")
        && matches!(
            bytes.get(8..12),
            Some(b"heic" | b"heix" | b"hevc" | b"hevx")
        )
    {
        Some("heic")
    } else {
        None
    }
}

pub fn read_image(path: &Path) -> Result<Vec<u8>> {
    let metadata = fs::symlink_metadata(path).context("FILE_MISSING")?;
    if !metadata.is_file() || metadata.file_type().is_symlink() || metadata.len() > 32 * 1024 * 1024
    {
        bail!("INVALID_MEDIA_CONTENT");
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if metadata.file_attributes() & 0x400 != 0 {
            bail!("INVALID_MEDIA_CONTENT");
        }
    }
    let bytes = fs::read(path).context("IO_ERROR")?;
    if image_extension(&bytes).is_none() {
        bail!("INVALID_MEDIA_CONTENT");
    }
    Ok(bytes)
}

pub fn validate_album(directory: &Path, files: &[AlbumFile]) -> Result<u64> {
    let metadata = fs::symlink_metadata(directory).context("FILE_MISSING")?;
    if !metadata.is_dir()
        || metadata.file_type().is_symlink()
        || files.is_empty()
        || files.len() > 64
    {
        bail!("INVALID_ENGINE_RESULT");
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if metadata.file_attributes() & 0x400 != 0 {
            bail!("INVALID_ENGINE_RESULT");
        }
    }
    let mut total = 0u64;
    for (index, file) in files.iter().enumerate() {
        let extension = file
            .filename
            .rsplit_once('.')
            .context("INVALID_ENGINE_RESULT")?
            .1;
        if !matches!(extension, "jpeg" | "png" | "webp" | "gif" | "heic" | "avif")
            || file.filename != format!("{:03}.{extension}", index + 1)
            || !matches!(file.quality_source.as_str(), "original" | "high_definition")
        {
            bail!("INVALID_ENGINE_RESULT");
        }
        let body = read_image(&directory.join(&file.filename))?;
        if image_extension(&body) != Some(extension)
            || body.len() as u64 != file.bytes
            || format!("{:x}", Sha256::digest(&body)) != file.sha256
        {
            bail!("INCOMPLETE_MEDIA");
        }
        total = total
            .checked_add(file.bytes)
            .context("INVALID_ENGINE_RESULT")?;
    }
    Ok(total)
}

fn move_album(source: &Path, target: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        use std::{ffi::CString, os::unix::ffi::OsStrExt};
        let source = CString::new(source.as_os_str().as_bytes()).context("IO_ERROR")?;
        let target = CString::new(target.as_os_str().as_bytes()).context("IO_ERROR")?;
        #[cfg(target_os = "macos")]
        let result =
            unsafe { libc::renamex_np(source.as_ptr(), target.as_ptr(), libc::RENAME_EXCL) };
        #[cfg(target_os = "linux")]
        let result = unsafe {
            libc::renameat2(
                libc::AT_FDCWD,
                source.as_ptr(),
                libc::AT_FDCWD,
                target.as_ptr(),
                libc::RENAME_NOREPLACE,
            )
        };
        #[cfg(not(any(target_os = "macos", target_os = "linux")))]
        let result = -1;
        if result != 0 {
            return Err(std::io::Error::last_os_error()).context("IO_ERROR");
        }
    }
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        let source: Vec<u16> = source.as_os_str().encode_wide().chain(Some(0)).collect();
        let target: Vec<u16> = target.as_os_str().encode_wide().chain(Some(0)).collect();
        // Flags 0: same-volume move, no overwrite of an existing directory.
        if unsafe {
            windows_sys::Win32::Storage::FileSystem::MoveFileExW(
                source.as_ptr(),
                target.as_ptr(),
                0,
            )
        } == 0
        {
            return Err(std::io::Error::last_os_error()).context("IO_ERROR");
        }
    }
    Ok(())
}

pub fn finalize_album(
    video: &Video,
    result: &Value,
    staging: &Path,
    target: &Path,
    before_move: impl FnOnce(&Path, u64, &[AlbumFile]) -> Result<()>,
) -> Result<Downloaded> {
    video.validate()?;
    if video.kind != "image_album"
        || result["kind"] != "image_album"
        || result["platform"] != video.platform
        || result["video_id"] != video.video_id
    {
        bail!("INVALID_ENGINE_RESULT");
    }
    let files: Vec<AlbumFile> =
        serde_json::from_value(result["files"].clone()).context("INVALID_ENGINE_RESULT")?;
    let source = staging.join("album");
    if fs::read_dir(&source)?.count() != files.len() {
        bail!("INCOMPLETE_MEDIA");
    }
    let size = validate_album(&source, &files)?;
    if result["bytes"].as_u64() != Some(size) {
        bail!("INCOMPLETE_MEDIA");
    }
    for file in &files {
        fs::File::open(source.join(&file.filename))?.sync_all()?;
    }
    before_move(&source, size, &files)?;
    move_album(&source, target)?;
    Ok(Downloaded {
        path: target.to_owned(),
        bytes: size,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn video() -> Video {
        Video {
            platform: "tiktok".into(),
            video_id: "123".into(),
            canonical_url: "unused".into(),
            kind: "video".into(),
            resolver: "yt-dlp".into(),
        }
    }
    fn result(size: usize) -> Value {
        json!({"platform":"tiktok","video_id":"123","container":"mp4","filename":"video.mp4","bytes":size})
    }
    #[test]
    fn saves_without_overwriting_and_rejects_bad_content() {
        let root = tempfile::tempdir().unwrap();
        let stage = tempfile::tempdir_in(root.path()).unwrap();
        let body = b"\x00\x00\x00\x18ftypmp42payload";
        fs::write(stage.path().join("video.mp4"), body).unwrap();
        let saved = finalize(&video(), &result(body.len()), stage.path(), root.path()).unwrap();
        assert_eq!(fs::read(&saved.path).unwrap(), body);
        fs::write(stage.path().join("video.mp4"), body).unwrap();
        assert!(finalize(&video(), &result(body.len()), stage.path(), root.path()).is_err());
        assert_eq!(fs::read(&saved.path).unwrap(), body);
        fs::write(stage.path().join("video.mp4"), b"<html>error page</html>").unwrap();
        assert!(finalize(&video(), &result(23), stage.path(), root.path()).is_err());
    }
    #[test]
    fn rejects_path_identity_and_size_mismatch() {
        let root = tempfile::tempdir().unwrap();
        for (key, value) in [
            ("filename", json!("../video.mp4")),
            ("platform", json!("douyin")),
            ("video_id", json!("456")),
            ("container", json!("exe")),
        ] {
            let mut bad = result(12);
            bad[key] = value;
            assert!(finalize(&video(), &bad, root.path(), root.path()).is_err());
        }
        fs::write(root.path().join("video.mp4"), b"\x00\x00\x00\x18ftypmp42").unwrap();
        assert!(finalize(&video(), &result(99), root.path(), root.path()).is_err());
        assert!(!root.path().join("tiktok_123.mp4").exists());
    }
    #[test]
    fn cover_commit_validates_content_path_size_and_never_overwrites() {
        let root = tempfile::tempdir().unwrap();
        let stage = tempfile::tempdir_in(root.path()).unwrap();
        let source = stage.path().join("cover");
        let target = root.path().join("saved.cover");
        let body = b"\xff\xd8\xffimage";
        fs::write(&source, body).unwrap();
        assert!(finalize_cover(
            &json!({"cover_filename":"../cover","cover_bytes":body.len()}),
            stage.path(),
            &target
        )
        .is_err());
        assert!(finalize_cover(
            &json!({"cover_filename":"cover","cover_bytes":100}),
            stage.path(),
            &target
        )
        .is_err());
        let result = json!({"cover_filename":"cover","cover_bytes":body.len()});
        let saved = finalize_cover(&result, stage.path(), &target).unwrap();
        fs::write(&source, body).unwrap();
        assert!(finalize_cover(&result, stage.path(), &target).is_err());
        assert_eq!(fs::read(&saved).unwrap(), body);
        fs::write(&source, b"<html>error</html>").unwrap();
        assert!(read_cover(&source).is_err());
    }
    #[test]
    fn album_commit_never_overwrites_and_requires_complete_evidence() {
        let root = tempfile::tempdir().unwrap();
        let stage = root.path().join("stage");
        fs::create_dir_all(stage.join("album")).unwrap();
        let body = b"\xff\xd8\xfffixture";
        fs::write(stage.join("album/001.jpeg"), body).unwrap();
        let mut identity = video();
        identity.kind = "image_album".into();
        identity.resolver = "dtk".into();
        let result = json!({"platform":"tiktok","video_id":"123","kind":"image_album","bytes":body.len(),
            "files":[{"filename":"001.jpeg","bytes":body.len(),"sha256":format!("{:x}",Sha256::digest(body)),"quality_source":"high_definition"}]});
        let target = root.path().join("album");
        assert!(
            finalize_album(&identity, &result, &stage, &target, |_, _, _| bail!(
                "DATABASE_ERROR"
            ))
            .is_err()
        );
        assert!(!target.exists());
        fs::create_dir(&target).unwrap();
        fs::write(target.join("user.txt"), "preserve").unwrap();
        assert!(finalize_album(&identity, &result, &stage, &target, |_, _, _| Ok(())).is_err());
        assert_eq!(fs::read(target.join("user.txt")).unwrap(), b"preserve");
        assert!(stage.join("album/001.jpeg").is_file());
        let target = root.path().join("new-album");
        let saved = finalize_album(&identity, &result, &stage, &target, |_, _, _| Ok(())).unwrap();
        let files: Vec<AlbumFile> = serde_json::from_value(result["files"].clone()).unwrap();
        assert_eq!(
            validate_album(&saved.path, &files).unwrap(),
            body.len() as u64
        );
        fs::write(saved.path.join("001.jpeg"), b"\xff\xd8\xffchanged").unwrap();
        assert!(validate_album(&saved.path, &files).is_err());
    }
}
