use super::profile_key;
use std::{
    io,
    os::unix::fs::{DirBuilderExt, FileTypeExt, MetadataExt, PermissionsExt},
    path::{Path, PathBuf},
};
use tokio::net::{UnixListener, UnixStream};

pub(super) struct Listener {
    endpoint: String,
    path: PathBuf,
    listener: UnixListener,
}
impl Listener {
    pub(super) async fn bind(profile: &Path) -> io::Result<Self> {
        // SAFETY: geteuid has no preconditions and does not retain any memory.
        let uid = unsafe { libc::geteuid() };
        // macOS sockaddr_un allows only 104 bytes, including the terminator.
        // Its per-user temp/profile directories can already exceed that limit.
        let directory = Path::new("/tmp").join(format!("yingqu-mcp-{uid}"));
        match std::fs::DirBuilder::new().mode(0o700).create(&directory) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error),
        }
        let metadata = std::fs::symlink_metadata(&directory)?;
        if !metadata.is_dir() || metadata.uid() != uid || metadata.mode() & 0o777 != 0o700 {
            return Err(io::ErrorKind::PermissionDenied.into());
        }
        let path = directory.join(format!("{}.sock", profile_key(profile, &uid.to_string())));
        match std::fs::symlink_metadata(&path) {
            Ok(metadata) => {
                if !metadata.file_type().is_socket() || metadata.uid() != uid {
                    return Err(io::ErrorKind::PermissionDenied.into());
                }
                match UnixStream::connect(&path).await {
                    Ok(_) => return Err(io::ErrorKind::AddrInUse.into()),
                    Err(error) if error.kind() == io::ErrorKind::ConnectionRefused => {
                        std::fs::remove_file(&path)?
                    }
                    Err(error) => return Err(error),
                }
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
        let listener = UnixListener::bind(&path)?;
        let bound = Self {
            endpoint: path.to_string_lossy().into_owned(),
            path,
            listener,
        };
        std::fs::set_permissions(&bound.path, std::fs::Permissions::from_mode(0o600))?;
        Ok(bound)
    }
    pub(super) fn endpoint(&self) -> &str {
        &self.endpoint
    }
    pub(super) async fn accept(&mut self) -> io::Result<UnixStream> {
        self.listener.accept().await.map(|(stream, _)| stream)
    }
}
impl Drop for Listener {
    fn drop(&mut self) {
        // Only the endpoint successfully bound by this listener is removed.
        if let Err(error) = std::fs::remove_file(&self.path) {
            if error.kind() != io::ErrorKind::NotFound {
                eprintln!("MCP IPC socket cleanup failed: {:?}", error.kind());
            }
        }
    }
}

#[cfg(test)]
#[tokio::test]
async fn private_socket_permissions_cleanup_and_stale_recovery() {
    let root = tempfile::tempdir().unwrap();
    let profile = root.path().join("test.sqlite");
    let listener = Listener::bind(&profile).await.unwrap();
    let path = listener.path.clone();
    assert_eq!(
        std::fs::metadata(path.parent().unwrap()).unwrap().mode() & 0o777,
        0o700
    );
    assert_eq!(std::fs::metadata(&path).unwrap().mode() & 0o777, 0o600);
    assert!(Listener::bind(&profile).await.is_err());
    drop(listener);
    assert!(!path.exists());
    let stale = std::os::unix::net::UnixListener::bind(&path).unwrap();
    drop(stale);
    assert!(path.exists());
    let recovered = Listener::bind(&profile).await.unwrap();
    drop(recovered);
    std::fs::write(&path, b"preserve regular file").unwrap();
    assert!(Listener::bind(&profile).await.is_err());
    assert_eq!(std::fs::read(&path).unwrap(), b"preserve regular file");
    std::fs::remove_file(&path).unwrap();
}

#[cfg(test)]
#[tokio::test]
async fn long_unicode_profiles_use_short_distinct_socket_paths() {
    let root = tempfile::tempdir().unwrap();
    let directory = root.path().join("资料库".repeat(15));
    std::fs::create_dir(&directory).unwrap();
    let first = directory.join("first.sqlite");
    let second = directory.join("second.sqlite");
    std::fs::write(&first, b"").unwrap();
    std::fs::write(&second, b"").unwrap();
    let first = first.canonicalize().unwrap();
    let second = second.canonicalize().unwrap();
    assert!(first.as_os_str().as_encoded_bytes().len() > 104);
    let first = Listener::bind(&first).await.unwrap();
    let second = Listener::bind(&second).await.unwrap();
    assert!(first.endpoint.len() < 104);
    assert!(second.endpoint.len() < 104);
    assert_ne!(first.endpoint, second.endpoint);
}
