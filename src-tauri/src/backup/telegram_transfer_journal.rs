//! Durable per-media send identity and account flood waits, without a second task queue.
use super::telegram_transfer::{Envelope, RemoteDocument, Target};
use crate::desktop_core::Core;
use anyhow::{bail, Context, Result};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum SendState {
    Prepared,
    Sending,
    Confirmed,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Entry {
    pub target: Target,
    pub envelope: Envelope,
    pub random_id: i64,
    pub state: SendState,
    pub document: Option<RemoteDocument>,
    pub last_error: Option<String>,
}
pub(super) trait Journal: Send + Sync {
    fn get(&self, _operation: &str, _kind: &str) -> Result<Option<Entry>> {
        Ok(None)
    }
    fn reserve(&self, entry: Entry) -> Result<Entry>;
    fn save(&self, entry: &Entry) -> Result<()>;
    fn cooldown(&self, account: &str) -> Result<i64>;
    fn failure(&self, account: &str, error: &str) -> Result<()>;
}
pub(super) struct CoreJournal<'a>(pub &'a Core);
impl Journal for CoreJournal<'_> {
    fn get(&self, operation: &str, kind: &str) -> Result<Option<Entry>> {
        self.0.backup_db(|db| get(db, operation, kind))
    }
    fn reserve(&self, entry: Entry) -> Result<Entry> {
        self.0.backup_db(|db| reserve(db, entry))
    }
    fn save(&self, entry: &Entry) -> Result<()> {
        self.0.backup_db(|db| save(db, entry))
    }
    fn cooldown(&self, account: &str) -> Result<i64> {
        self.0.backup_db(|db| cooldown(db, account))
    }
    fn failure(&self, account: &str, error: &str) -> Result<()> {
        self.0.backup_db(|db| failure(db, account, error))
    }
}
pub(super) fn get(db: &Connection, operation: &str, kind: &str) -> Result<Option<Entry>> {
    let raw: Option<String> = db
        .query_row(
            "SELECT entry_json FROM telegram_media_sends WHERE operation_id=?1 AND kind=?2",
            params![operation, kind],
            |r| r.get(0),
        )
        .optional()?;
    raw.map(|value| serde_json::from_str(&value).context("TELEGRAM_SEND_EVIDENCE_INVALID"))
        .transpose()
}
pub(super) fn reserve(db: &Connection, entry: Entry) -> Result<Entry> {
    let tx = db.unchecked_transaction()?;
    tx.execute(
        "INSERT OR IGNORE INTO telegram_media_sends VALUES(?1,?2,?3)",
        params![
            entry.envelope.operation_id,
            entry.envelope.kind,
            serde_json::to_string(&entry)?
        ],
    )?;
    let raw: String = tx.query_row(
        "SELECT entry_json FROM telegram_media_sends WHERE operation_id=?1 AND kind=?2",
        params![entry.envelope.operation_id, entry.envelope.kind],
        |r| r.get(0),
    )?;
    let saved: Entry = serde_json::from_str(&raw).context("TELEGRAM_SEND_EVIDENCE_INVALID")?;
    if saved.target != entry.target
        || saved.envelope != entry.envelope
        || saved.random_id != entry.random_id
    {
        bail!("TELEGRAM_OPERATION_MISMATCH");
    }
    tx.commit()?;
    Ok(saved)
}
pub(super) fn save(db: &Connection, entry: &Entry) -> Result<()> {
    if db.execute(
        "UPDATE telegram_media_sends SET entry_json=?3 WHERE operation_id=?1 AND kind=?2",
        params![
            entry.envelope.operation_id,
            entry.envelope.kind,
            serde_json::to_string(entry)?
        ],
    )? != 1
    {
        bail!("TELEGRAM_SEND_EVIDENCE_INVALID");
    }
    Ok(())
}
pub(super) fn cooldown(db: &Connection, account: &str) -> Result<i64> {
    Ok(db
        .query_row(
            "SELECT retry_at FROM telegram_transfer_cooldowns WHERE account_id=?",
            [account],
            |r| r.get(0),
        )
        .optional()?
        .unwrap_or(0))
}
pub(super) fn failure(db: &Connection, account: &str, error: &str) -> Result<()> {
    if let Some(seconds) = error
        .strip_prefix("TELEGRAM_FLOOD_WAIT:")
        .and_then(|v| v.parse::<u32>().ok())
    {
        let retry = super::telegram_transfer::now().saturating_add(i64::from(seconds));
        db.execute("INSERT INTO telegram_transfer_cooldowns VALUES(?1,?2) ON CONFLICT(account_id) DO UPDATE SET retry_at=MAX(retry_at,excluded.retry_at)",params![account,retry])?;
    }
    Ok(())
}
