//! Dedicated grammers session storage; authorization keys never enter SQLite or session files.
use anyhow::{bail, Context, Result};
use grammers_session::{
    storages::MemorySession,
    types::{DcOption, PeerId, PeerInfo, UpdateState, UpdatesState},
    BoxFuture, Session, SessionData,
};
use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex};
use zeroize::Zeroizing;

const SERVICE: &str = "com.yingqu.desktop.telegram-storage";

#[derive(Debug)]
pub(super) struct SessionError(anyhow::Error);
impl std::fmt::Display for SessionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("TELEGRAM_SESSION_STORAGE")
    }
}
impl std::error::Error for SessionError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(self.0.as_ref())
    }
}
impl From<anyhow::Error> for SessionError {
    fn from(error: anyhow::Error) -> Self {
        Self(error)
    }
}
impl From<grammers_session::storages::MemorySessionError> for SessionError {
    fn from(error: grammers_session::storages::MemorySessionError) -> Self {
        Self(error.into())
    }
}
type StorageResult<T> = std::result::Result<T, SessionError>;

pub(super) trait SessionVault: Send + Sync {
    fn read(&self, key: &str) -> Result<Option<Zeroizing<String>>>;
    fn write(&self, key: &str, value: &str) -> Result<()>;
    fn delete(&self, key: &str) -> Result<()>;
}
pub(super) struct SystemSessionVault;
impl SessionVault for SystemSessionVault {
    fn read(&self, key: &str) -> Result<Option<Zeroizing<String>>> {
        match keyring::Entry::new(SERVICE, key)
            .context("TELEGRAM_SESSION_STORAGE")?
            .get_password()
        {
            Ok(value) => Ok(Some(Zeroizing::new(value))),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(_) => bail!("TELEGRAM_SESSION_STORAGE"),
        }
    }
    fn write(&self, key: &str, value: &str) -> Result<()> {
        keyring::Entry::new(SERVICE, key)
            .context("TELEGRAM_SESSION_STORAGE")?
            .set_password(value)
            .map_err(|_| anyhow::anyhow!("TELEGRAM_SESSION_STORAGE"))
    }
    fn delete(&self, key: &str) -> Result<()> {
        match keyring::Entry::new(SERVICE, key)
            .context("TELEGRAM_SESSION_STORAGE")?
            .delete_credential()
        {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(_) => bail!("TELEGRAM_SESSION_STORAGE"),
        }
    }
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Metadata {
    version: u32,
    revoked: bool,
    home_dc: i32,
    dc_ids: Vec<i32>,
    self_peer: Option<PeerInfo>,
}

pub(super) struct VaultSession {
    pub(super) file_authorization: Arc<crate::telegram_transfer_engine::Authorization>,
    memory: MemorySession,
    metadata: Mutex<Metadata>,
    writes: tokio::sync::Mutex<()>,
    vault: Arc<dyn SessionVault>,
    namespace: String,
}
impl VaultSession {
    pub(super) async fn load(namespace: String, vault: Arc<dyn SessionVault>) -> Result<Arc<Self>> {
        let (metadata, data) = {
            let vault = vault.clone();
            let namespace = namespace.clone();
            tokio::task::spawn_blocking(move || {
                let mut data = SessionData::default();
                let metadata = match vault.read(&namespace)? {
                    Some(raw) => serde_json::from_str::<Metadata>(&raw)
                        .map_err(|_| anyhow::anyhow!("TELEGRAM_SESSION_INVALID"))?,
                    None => Metadata {
                        version: 1,
                        revoked: false,
                        home_dc: data.home_dc,
                        dc_ids: vec![],
                        self_peer: None,
                    },
                };
                if metadata.version != 1 || metadata.dc_ids.len() > 32 {
                    bail!("TELEGRAM_SESSION_INVALID");
                }
                if metadata.revoked {
                    return Ok((metadata, data));
                }
                data.home_dc = metadata.home_dc;
                for dc_id in &metadata.dc_ids {
                    let raw = vault
                        .read(&format!("{namespace}/dc/{dc_id}"))?
                        .context("TELEGRAM_SESSION_INVALID")?;
                    let option: DcOption = serde_json::from_str(&raw)
                        .map_err(|_| anyhow::anyhow!("TELEGRAM_SESSION_INVALID"))?;
                    if option.id != *dc_id {
                        bail!("TELEGRAM_SESSION_INVALID");
                    }
                    data.dc_options.insert(*dc_id, option);
                }
                if let Some(peer) = &metadata.self_peer {
                    if !matches!(
                        peer,
                        PeerInfo::User {
                            is_self: Some(true),
                            ..
                        }
                    ) {
                        bail!("TELEGRAM_SESSION_INVALID");
                    }
                    data.peer_infos.insert(peer.id(), peer.clone());
                }
                Ok::<_, anyhow::Error>((metadata, data))
            })
            .await
            .context("TELEGRAM_SESSION_STORAGE")??
        };
        Ok(Arc::new(Self {
            file_authorization: Arc::default(),
            memory: MemorySession::from(data),
            metadata: Mutex::new(metadata),
            writes: tokio::sync::Mutex::new(()),
            vault,
            namespace,
        }))
    }
    async fn persist(&self, metadata: Metadata, dc: Option<DcOption>) -> Result<()> {
        let vault = self.vault.clone();
        let namespace = self.namespace.clone();
        // Each DC has its own small entry, below Windows Credential Manager's blob limit.
        // Write the key before its manifest. A failed write must never replace a usable manifest.
        // These short native writes must finish before cancellation can release the write lock.
        if let Some(dc) = dc {
            let raw = Zeroizing::new(serde_json::to_string(&dc)?);
            vault.write(&format!("{namespace}/dc/{}", dc.id), &raw)?;
        }
        let raw = Zeroizing::new(serde_json::to_string(&metadata)?);
        vault.write(&namespace, &raw)?;
        *self
            .metadata
            .lock()
            .expect("telegram metadata lock poisoned") = metadata;
        Ok(())
    }
    pub(super) fn account_id(&self) -> Option<i64> {
        let metadata = self
            .metadata
            .lock()
            .expect("telegram metadata lock poisoned");
        if metadata.revoked {
            return None;
        }
        metadata
            .self_peer
            .as_ref()
            .map(|peer| peer.id().bare_id_unchecked())
    }
    pub(super) fn revoked(&self) -> bool {
        self.metadata
            .lock()
            .expect("telegram metadata lock poisoned")
            .revoked
    }
    pub(super) async fn erase(&self) -> Result<()> {
        let _writes = self.writes.lock().await;
        let mut metadata = self
            .metadata
            .lock()
            .expect("telegram metadata lock poisoned")
            .clone();
        metadata.revoked = true;
        self.persist(metadata, None).await?;
        let vault = self.vault.clone();
        let namespace = self.namespace.clone();
        let ids = self
            .metadata
            .lock()
            .expect("telegram metadata lock poisoned")
            .dc_ids
            .clone();
        for id in ids {
            vault.delete(&format!("{namespace}/dc/{id}"))?;
        }
        // Keep the manifest if any secret could not be deleted, so cleanup remains retryable.
        vault.delete(&namespace)
    }
}
impl Session for VaultSession {
    type Error = SessionError;
    fn home_dc_id(&self) -> StorageResult<i32> {
        Ok(self.memory.home_dc_id()?)
    }
    fn dc_option(&self, id: i32) -> StorageResult<Option<DcOption>> {
        Ok(self.memory.dc_option(id)?)
    }
    fn set_home_dc_id(&self, id: i32) -> BoxFuture<'_, StorageResult<()>> {
        Box::pin(async move {
            let _writes = self.writes.lock().await;
            let mut metadata = self
                .metadata
                .lock()
                .expect("telegram metadata lock poisoned")
                .clone();
            metadata.home_dc = id;
            self.persist(metadata, None).await?;
            Ok(self.memory.set_home_dc_id(id).await?)
        })
    }
    fn set_dc_option(&self, dc: &DcOption) -> BoxFuture<'_, StorageResult<()>> {
        let dc = dc.clone();
        Box::pin(async move {
            let _writes = self.writes.lock().await;
            let mut metadata = self
                .metadata
                .lock()
                .expect("telegram metadata lock poisoned")
                .clone();
            if !metadata.dc_ids.contains(&dc.id) {
                metadata.dc_ids.push(dc.id);
            }
            self.persist(metadata, Some(dc.clone())).await?;
            Ok(self.memory.set_dc_option(&dc).await?)
        })
    }
    fn peer(&self, peer: PeerId) -> BoxFuture<'_, StorageResult<Option<PeerInfo>>> {
        Box::pin(async move { Ok(self.memory.peer(peer).await?) })
    }
    fn cache_peer(&self, peer: PeerInfo) -> BoxFuture<'_, StorageResult<()>> {
        Box::pin(async move {
            if matches!(
                &peer,
                PeerInfo::User {
                    is_self: Some(true),
                    ..
                }
            ) {
                let _writes = self.writes.lock().await;
                let mut metadata = self
                    .metadata
                    .lock()
                    .expect("telegram metadata lock poisoned")
                    .clone();
                if metadata
                    .self_peer
                    .as_ref()
                    .is_some_and(|old| old.id() != peer.id())
                {
                    return Err(anyhow::anyhow!("TELEGRAM_ACCOUNT_MISMATCH").into());
                }
                metadata.self_peer = Some(peer.clone());
                self.persist(metadata, None).await?;
            }
            Ok(self.memory.cache_peer(peer).await?)
        })
    }
    fn updates_state(&self) -> BoxFuture<'_, StorageResult<UpdatesState>> {
        Box::pin(async move { Ok(self.memory.updates_state().await?) })
    }
    fn set_update_state(&self, update: UpdateState) -> BoxFuture<'_, StorageResult<()>> {
        // Backup discovery uses its own durable history cursor, not this optional update cache.
        Box::pin(async move { Ok(self.memory.set_update_state(update).await?) })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        collections::HashMap,
        sync::atomic::{AtomicBool, Ordering},
    };
    #[derive(Default)]
    struct Vault {
        values: Mutex<HashMap<String, String>>,
        fail: AtomicBool,
        fail_delete: Mutex<Option<String>>,
    }
    impl SessionVault for Vault {
        fn read(&self, key: &str) -> Result<Option<Zeroizing<String>>> {
            Ok(self
                .values
                .lock()
                .unwrap()
                .get(key)
                .cloned()
                .map(Zeroizing::new))
        }
        fn write(&self, key: &str, value: &str) -> Result<()> {
            if self.fail.load(Ordering::SeqCst) {
                bail!("TELEGRAM_SESSION_STORAGE");
            }
            assert!(
                value.encode_utf16().count() * 2 <= 2560,
                "credential entry exceeds Windows limit"
            );
            self.values.lock().unwrap().insert(key.into(), value.into());
            Ok(())
        }
        fn delete(&self, key: &str) -> Result<()> {
            if self.fail_delete.lock().unwrap().as_deref() == Some(key) {
                bail!("TELEGRAM_SESSION_STORAGE");
            }
            self.values.lock().unwrap().remove(key);
            Ok(())
        }
    }
    #[tokio::test]
    async fn telegram_vault_restores_dc_keys_and_self_without_plaintext_files() {
        let vault = Arc::new(Vault::default());
        let session = VaultSession::load("fixture".into(), vault.clone())
            .await
            .unwrap();
        for id in 1..=5 {
            let mut dc = session.dc_option(id).unwrap().unwrap();
            dc.auth_key = Some([id as u8; 256]);
            session.set_dc_option(&dc).await.unwrap();
        }
        session.set_home_dc_id(4).await.unwrap();
        session
            .cache_peer(PeerInfo::User {
                id: 123,
                auth: None,
                bot: Some(false),
                is_self: Some(true),
            })
            .await
            .unwrap();
        session
            .cache_peer(PeerInfo::User {
                id: 999,
                auth: None,
                bot: None,
                is_self: Some(false),
            })
            .await
            .unwrap();
        drop(session);
        let restored = VaultSession::load("fixture".into(), vault.clone())
            .await
            .unwrap();
        assert_eq!(restored.home_dc_id().unwrap(), 4);
        assert_eq!(restored.account_id(), Some(123));
        assert_eq!(
            restored.dc_option(3).unwrap().unwrap().auth_key,
            Some([3; 256])
        );
        assert!(restored
            .peer(PeerId::user_unchecked(999))
            .await
            .unwrap()
            .is_none());
        assert!(restored
            .cache_peer(PeerInfo::User {
                id: 321,
                auth: None,
                bot: None,
                is_self: Some(true)
            })
            .await
            .is_err());
        restored.erase().await.unwrap();
        assert!(vault.values.lock().unwrap().is_empty());
    }
    #[tokio::test]
    async fn telegram_vault_errors_do_not_replace_live_or_saved_home_dc() {
        let vault = Arc::new(Vault::default());
        let session = VaultSession::load("fixture".into(), vault.clone())
            .await
            .unwrap();
        session.set_home_dc_id(3).await.unwrap();
        vault.fail.store(true, Ordering::SeqCst);
        assert!(session.set_home_dc_id(4).await.is_err());
        assert_eq!(session.home_dc_id().unwrap(), 3);
        let restored = VaultSession::load("fixture".into(), vault).await.unwrap();
        assert_eq!(restored.home_dc_id().unwrap(), 3);
    }
    #[tokio::test]
    async fn telegram_vault_account_binding_failure_is_not_a_saved_login() {
        let vault = Arc::new(Vault::default());
        let session = VaultSession::load("fixture".into(), vault.clone())
            .await
            .unwrap();
        vault.fail.store(true, Ordering::SeqCst);
        assert!(session
            .cache_peer(PeerInfo::User {
                id: 123,
                auth: None,
                bot: Some(false),
                is_self: Some(true)
            })
            .await
            .is_err());
        assert!(session.account_id().is_none());
        let restarted = VaultSession::load("fixture".into(), vault).await.unwrap();
        assert!(
            restarted.account_id().is_none(),
            "failed keychain write must not become a saved login"
        );
    }
    #[tokio::test]
    async fn telegram_vault_missing_key_is_an_error_and_namespaces_are_isolated() {
        let vault = Arc::new(Vault::default());
        let session = VaultSession::load("first".into(), vault.clone())
            .await
            .unwrap();
        let mut dc = session.dc_option(2).unwrap().unwrap();
        dc.auth_key = Some([7; 256]);
        session.set_dc_option(&dc).await.unwrap();
        let other = VaultSession::load("second".into(), vault.clone())
            .await
            .unwrap();
        assert!(other.dc_option(2).unwrap().unwrap().auth_key.is_none());
        vault.delete("first/dc/2").unwrap();
        assert!(VaultSession::load("first".into(), vault).await.is_err());
    }
    #[tokio::test]
    async fn telegram_vault_partial_logout_cleanup_survives_restart_and_is_retryable() {
        let vault = Arc::new(Vault::default());
        let session = VaultSession::load("fixture".into(), vault.clone())
            .await
            .unwrap();
        for id in 1..=3 {
            let mut dc = session.dc_option(id).unwrap().unwrap();
            dc.auth_key = Some([id as u8; 256]);
            session.set_dc_option(&dc).await.unwrap();
        }
        *vault.fail_delete.lock().unwrap() = Some("fixture/dc/2".into());
        assert!(session.erase().await.is_err());
        assert!(!vault.values.lock().unwrap().contains_key("fixture/dc/1"));
        drop(session);
        let restored = VaultSession::load("fixture".into(), vault.clone())
            .await
            .unwrap();
        assert!(restored.revoked());
        assert!(restored.account_id().is_none());
        assert!(
            restored.dc_option(3).unwrap().unwrap().auth_key.is_none(),
            "revoked key must never be reconnected"
        );
        *vault.fail_delete.lock().unwrap() = None;
        restored.erase().await.unwrap();
        assert!(vault.values.lock().unwrap().is_empty());
    }
}
