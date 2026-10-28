use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File},
    io::{Read, Write},
    path::{Path, PathBuf},
};

#[derive(Clone, Copy, Debug, Serialize)]
pub struct TransferProgress {
    pub phase: &'static str,
    pub completed_files: usize,
    pub total_files: usize,
    pub bytes: u64,
}
pub type TransferCallback<'a> = dyn FnMut(TransferProgress) -> Result<()> + 'a;

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Payload {
    pub path: String,
    pub size: u64,
    pub sha256: String,
}

pub(super) fn regular(path: &Path) -> Result<()> {
    let meta = fs::symlink_metadata(path).context("IO_ERROR")?;
    if !meta.is_file() || meta.file_type().is_symlink() {
        bail!("LIBRARY_PACKAGE_PATH");
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if meta.file_attributes() & 0x400 != 0 {
            bail!("LIBRARY_PACKAGE_PATH");
        }
    }
    Ok(())
}

/// Payload names are generated independently of IDs, titles and original names.
pub(super) fn payload_path(root: &Path, relative: &str) -> Result<PathBuf> {
    let number = relative
        .strip_prefix("files/")
        .and_then(|s| s.strip_suffix(".bin"))
        .context("LIBRARY_PACKAGE_PATH")?;
    if number.len() != 8 || !number.bytes().all(|c| c.is_ascii_digit()) {
        bail!("LIBRARY_PACKAGE_PATH");
    }
    let path = root.join("files").join(format!("{number}.bin"));
    regular(&path)?;
    let resolved = path.canonicalize().context("IO_ERROR")?;
    if !resolved.starts_with(root) {
        bail!("LIBRARY_PACKAGE_PATH");
    }
    Ok(resolved)
}

pub(super) fn new_target(target: &Path) -> Result<PathBuf> {
    let name = target.file_name().context("INVALID_INPUT")?;
    if target
        .components()
        .any(|c| matches!(c, std::path::Component::ParentDir))
    {
        bail!("INVALID_INPUT");
    }
    let resolved = target
        .parent()
        .context("INVALID_INPUT")?
        .canonicalize()
        .context("IO_ERROR")?
        .join(name);
    match fs::symlink_metadata(&resolved) {
        Ok(_) => bail!("LIBRARY_TARGET_EXISTS"),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(resolved),
        Err(error) => Err(error).context("IO_ERROR"),
    }
}

pub(super) fn publish(stage: &Path, target: &Path) -> Result<()> {
    new_target(target)?;
    fs::rename(stage, target).context("IO_ERROR")
}

pub(super) fn copy_hash(
    input: &mut impl Read,
    target: Option<&Path>,
    progress: &mut TransferProgress,
    notify: &mut TransferCallback<'_>,
) -> Result<(u64, String)> {
    let mut output = target
        .map(|p| File::create_new(p).context("IO_ERROR"))
        .transpose()?;
    let mut hash = Sha256::new();
    let mut size = 0;
    let mut buffer = [0u8; 64 * 1024];
    loop {
        notify(*progress)?;
        let read = input.read(&mut buffer).context("IO_ERROR")?;
        if read == 0 {
            break;
        }
        if let Some(output) = &mut output {
            output.write_all(&buffer[..read]).context("IO_ERROR")?;
        }
        hash.update(&buffer[..read]);
        size += read as u64;
        progress.bytes += read as u64;
    }
    if let Some(output) = &mut output {
        output.sync_all().context("IO_ERROR")?;
    }
    Ok((size, format!("{:x}", hash.finalize())))
}

pub(super) fn filename(path: &Path) -> String {
    path.file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned()
}
pub(super) fn media_name(index: usize, original: &str) -> String {
    let extension = Path::new(original)
        .extension()
        .and_then(|s| s.to_str())
        .filter(|s| {
            [
                "mp4", "mov", "mkv", "webm", "m4v", "avi", "flv", "ts", "mpeg", "mpg", "wmv",
                "ogv", "3gp",
            ]
            .iter()
            .any(|ext| s.eq_ignore_ascii_case(ext))
        })
        .unwrap_or("mp4");
    format!("media/{index:08}.{extension}")
}
