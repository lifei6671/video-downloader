//! Lossless, deterministic album documents shared by S3 and Telegram.
use super::{model::Asset, repository};
use crate::files::AlbumFile;
use anyhow::{ensure, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
};

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    version: u32,
    files: Vec<AlbumFile>,
}

pub fn is_album(asset: &Asset) -> bool {
    asset.metadata["kind"] == "image_album"
}

pub fn files(metadata: &Value) -> Result<Vec<AlbumFile>> {
    let files: Vec<AlbumFile> =
        serde_json::from_value(metadata["album_files"].clone()).context("BACKUP_INVALID_ALBUM")?;
    validate_manifest(&files)?;
    Ok(files)
}

pub fn validate_manifest(files: &[AlbumFile]) -> Result<()> {
    ensure!(
        !files.is_empty() && files.len() <= 64,
        "BACKUP_INVALID_ALBUM"
    );
    for (index, file) in files.iter().enumerate() {
        if let Some(video) = &file.video {
            ensure!(
                video.filename == format!("{:03}.mp4", index + 1),
                "BACKUP_INVALID_ALBUM"
            );
        }
        let ext = file
            .filename
            .rsplit_once('.')
            .context("BACKUP_INVALID_ALBUM")?
            .1;
        ensure!(
            matches!(ext, "jpeg" | "png" | "webp" | "gif" | "heic" | "avif")
                && file.filename == format!("{:03}.{ext}", index + 1)
                && matches!(file.quality_source.as_str(), "original" | "high_definition"),
            "BACKUP_INVALID_ALBUM"
        );
        for (_, bytes, hash) in entries(std::slice::from_ref(file)) {
            ensure!(
                bytes > 0
                    && bytes <= 32 * 1024 * 1024
                    && hash.len() == 64
                    && hash.bytes().all(|b| b.is_ascii_hexdigit()),
                "BACKUP_INVALID_ALBUM"
            );
        }
    }
    Ok(())
}

fn entries(files: &[AlbumFile]) -> impl Iterator<Item = (&str, u64, &str)> {
    files.iter().flat_map(|f| {
        std::iter::once((f.filename.as_str(), f.bytes, f.sha256.as_str())).chain(
            f.video
                .iter()
                .map(|v| (v.filename.as_str(), v.bytes, v.sha256.as_str())),
        )
    })
}

pub fn present(asset: &Asset) -> bool {
    asset.path.as_ref().is_some_and(|p| {
        if is_album(asset) {
            p.is_dir()
        } else {
            p.is_file()
        }
    })
}

pub fn source(path: &Path, files: &[AlbumFile], archive_bytes: u64) -> Result<Value> {
    let members = entries(files)
        .map(|(name, _, _)| {
            serde_json::to_value(crate::desktop_core::transcription::Source::read(
                &path.join(name),
            )?)
            .map_err(Into::into)
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(
        json!({"path":path.canonicalize()?.to_string_lossy(), "size":archive_bytes, "members":members}),
    )
}

struct Checked<'a, R, F> {
    reader: R,
    stopped: &'a F,
}
impl<R: Read, F: Fn() -> bool> Read for Checked<'_, R, F> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        if (self.stopped)() {
            return Err(std::io::Error::other("CANCELED"));
        }
        self.reader.read(buf)
    }
}

fn append(
    builder: &mut tar::Builder<&mut fs::File>,
    name: &str,
    bytes: u64,
    reader: impl Read,
) -> Result<()> {
    let mut header = tar::Header::new_ustar();
    header.set_size(bytes);
    header.set_mode(0o644);
    header.set_entry_type(tar::EntryType::Regular);
    header.set_cksum();
    builder.append_data(&mut header, name, reader)?;
    Ok(())
}

fn directory(path: &Path) -> Result<()> {
    let meta = fs::symlink_metadata(path).context("FILE_MISSING")?;
    ensure!(
        meta.is_dir() && !meta.file_type().is_symlink(),
        "BACKUP_UNSAFE_FILE"
    );
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        ensure!(meta.file_attributes() & 0x400 == 0, "BACKUP_UNSAFE_FILE");
    }
    Ok(())
}

fn read_member(
    path: &Path,
    bytes: u64,
    hash: &str,
    stopped: &impl Fn() -> bool,
) -> Result<Vec<u8>> {
    ensure!(!stopped(), "CANCELED");
    let meta = super::restore::regular(path)?.context("FILE_MISSING")?;
    ensure!(meta.len() == bytes, "INCOMPLETE_MEDIA");
    let mut body = Vec::with_capacity(bytes as usize);
    Checked {
        reader: fs::File::open(path)?,
        stopped,
    }
    .take(bytes + 1)
    .read_to_end(&mut body)?;
    ensure!(
        body.len() as u64 == bytes && format!("{:x}", Sha256::digest(&body)) == hash,
        "INCOMPLETE_MEDIA"
    );
    let ext = path
        .extension()
        .and_then(|v| v.to_str())
        .context("BACKUP_INVALID_ALBUM")?;
    let image = crate::files::image_extension(&body);
    ensure!(
        if ext == "mp4" {
            body.len() >= 12 && crate::files::video_header_valid(ext, &body) && image.is_none()
        } else {
            image == Some(ext)
        },
        "INVALID_MEDIA_CONTENT"
    );
    Ok(body)
}

pub fn verify(path: &Path, files: &[AlbumFile], stopped: impl Fn() -> bool) -> Result<()> {
    validate_manifest(files)?;
    directory(path)?;
    for (name, bytes, hash) in entries(files) {
        read_member(&path.join(name), bytes, hash, &stopped)?;
    }
    Ok(())
}

pub fn pack(
    path: &Path,
    files: &[AlbumFile],
    stopped: impl Fn() -> bool,
) -> Result<tempfile::NamedTempFile> {
    ensure!(!stopped(), "CANCELED");
    validate_manifest(files)?;
    directory(path)?;
    let manifest = serde_json::to_vec(&Manifest {
        version: 1,
        files: files.to_vec(),
    })?;
    let mut result = tempfile::NamedTempFile::new()?;
    {
        let mut builder = tar::Builder::new(result.as_file_mut());
        append(
            &mut builder,
            "album.json",
            manifest.len() as u64,
            manifest.as_slice(),
        )?;
        for (name, bytes, hash) in entries(files) {
            let body = read_member(&path.join(name), bytes, hash, &stopped)?;
            append(&mut builder, name, bytes, body.as_slice())?;
        }
        builder.finish()?;
    }
    result.flush()?;
    ensure!(!stopped(), "CANCELED");
    Ok(result)
}

pub fn fingerprint(asset: &Asset, stopped: impl Fn() -> bool) -> Result<(String, Value)> {
    let path = asset.path.as_deref().context("FILE_MISSING")?;
    let files = files(&asset.metadata)?;
    let archive = pack(path, &files, &stopped)?;
    let (hash, raw) = repository::fingerprint_with_cancel(archive.path(), &stopped)?;
    Ok((
        hash,
        source(
            path,
            &files,
            raw["size"].as_u64().context("BACKUP_INVALID_SIZE")?,
        )?,
    ))
}

pub fn extract(
    archive: &Path,
    files: &[AlbumFile],
    directory: &Path,
    stopped: impl Fn() -> bool,
) -> Result<()> {
    validate_manifest(files)?;
    let mut archive = tar::Archive::new(Checked {
        reader: fs::File::open(archive)?,
        stopped: &stopped,
    });
    let mut contents = archive.entries()?;
    let mut first = contents.next().context("BACKUP_INVALID_ALBUM")??;
    ensure!(
        first.header().entry_type().is_file()
            && first.path()?.as_ref() == Path::new("album.json")
            && first.size() <= 256 * 1024,
        "BACKUP_INVALID_ALBUM"
    );
    let manifest: Manifest = serde_json::from_reader(&mut first).context("BACKUP_INVALID_ALBUM")?;
    ensure!(
        manifest.version == 1
            && serde_json::to_value(&manifest.files)? == serde_json::to_value(files)?,
        "BACKUP_INVALID_ALBUM"
    );
    drop(first);
    // Only write the manifest's fixed, validated member names into a new private directory.
    for (name, bytes, _) in entries(files) {
        let mut entry = contents.next().context("BACKUP_INVALID_ALBUM")??;
        ensure!(
            entry.header().entry_type().is_file()
                && entry.path()?.as_ref() == Path::new(name)
                && entry.size() == bytes,
            "BACKUP_INVALID_ALBUM"
        );
        let mut output = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(directory.join(name))?;
        ensure!(
            std::io::copy(&mut entry, &mut output)? == bytes,
            "BACKUP_SIZE_MISMATCH"
        );
        output.sync_all()?;
    }
    ensure!(contents.next().is_none(), "BACKUP_INVALID_ALBUM");
    ensure!(!stopped(), "CANCELED");
    verify(directory, files, &stopped)?;
    Ok(())
}

pub fn remove(path: &Path, files: &[AlbumFile]) -> Result<()> {
    validate_manifest(files)?;
    if !path.exists() {
        return Ok(());
    }
    directory(path)?;
    let mut remaining = vec![];
    for (name, bytes, hash) in entries(files) {
        let member = path.join(name);
        if super::restore::regular(&member)?.is_some() {
            read_member(&member, bytes, hash, &|| false)?;
            remaining.push(member);
        }
    }
    for member in remaining {
        fs::remove_file(member)?;
    }
    // Preserve any extra files added by the user.
    match fs::remove_dir(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::DirectoryNotEmpty => Ok(()),
        Err(error) => Err(error.into()),
    }
}

pub fn member(asset: &Asset, index: usize, video: bool) -> Result<PathBuf> {
    let files = files(&asset.metadata)?;
    let file = files.get(index).context("FILE_MISSING")?;
    let (name, bytes) = if video {
        let video = file
            .video
            .as_ref()
            .context("FILE_MISSING")?;
        (&video.filename, video.bytes)
    } else {
        (&file.filename, file.bytes)
    };
    let root = asset.path.as_ref().context("FILE_MISSING")?;
    directory(root)?;
    let path = root.join(name);
    let meta = super::restore::regular(&path)?.context("FILE_MISSING")?;
    ensure!(meta.len() == bytes, "FILE_MISSING");
    Ok(path)
}
