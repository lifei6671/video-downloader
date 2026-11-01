//! Persistent message locations for a channel, synchronized using Telegram's channel pts.
use super::*;
use crate::{backup::delete_timing, desktop_core::Core};
use rusqlite::{params, Connection, OptionalExtension};
use std::collections::BTreeSet;

impl ChannelSnapshot {
    pub(in crate::backup) fn has_operation(&self, operation: &str) -> Result<bool> {
        for doc in self
            .media
            .iter()
            .chain(self.indexes.iter().map(|i| &i.document))
        {
            let envelope: Envelope = serde_json::from_str(&doc.caption)?;
            if envelope.operation_id == operation {
                return Ok(true);
            }
        }
        Ok(false)
    }
}

#[derive(Debug, PartialEq, Eq)]
enum ChannelChanges {
    Delta(i32, BTreeSet<i32>),
    Unusable(&'static str),
}

/// A non-final or unrecognized difference cannot prove completeness for a destructive operation.
fn changed_messages(
    difference: tl::enums::updates::ChannelDifference,
    channel: i64,
    previous: i32,
) -> ChannelChanges {
    use tl::enums::{updates::ChannelDifference, Update};
    let mut ids = BTreeSet::new();
    let pts = match difference {
        ChannelDifference::Empty(d) if d.r#final => d.pts,
        ChannelDifference::Difference(d) if d.r#final => {
            for message in d.new_messages {
                ids.insert(message.id());
            }
            for update in d.other_updates {
                match update {
                    Update::MessageId(receipt) => {
                        ids.insert(receipt.id);
                    }
                    Update::EditChannelMessage(edit) => {
                        ids.insert(edit.message.id());
                    }
                    Update::NewChannelMessage(new) => {
                        ids.insert(new.message.id());
                    }
                    Update::DeleteChannelMessages(deleted) if deleted.channel_id == channel => {
                        ids.extend(deleted.messages);
                    }
                    Update::DeleteChannelMessages(_) => {
                        return ChannelChanges::Unusable("delete_channel_mismatch")
                    }
                    other => {
                        // The boxed TL constructor is the first word. Only its schema name is recorded.
                        let encoded = tl::Serializable::to_bytes(&other);
                        let id = u32::from_le_bytes(
                            encoded[..4].try_into().expect("boxed TL constructor"),
                        );
                        return ChannelChanges::Unusable(tl::name_for_id(id));
                    }
                }
            }
            d.pts
        }
        ChannelDifference::TooLong(_) => return ChannelChanges::Unusable("difference_too_long"),
        ChannelDifference::Empty(_) => return ChannelChanges::Unusable("empty_not_final"),
        ChannelDifference::Difference(_) => {
            return ChannelChanges::Unusable("difference_not_final")
        }
    };
    if pts < previous {
        ChannelChanges::Unusable("pts_regressed")
    } else if pts == previous && !ids.is_empty() {
        ChannelChanges::Unusable("changed_ids_without_pts_advance")
    } else {
        ChannelChanges::Delta(pts, ids)
    }
}

fn load(db: &Connection, space: &str, high: i64) -> Result<ChannelSnapshot> {
    let mut snapshot = ChannelSnapshot {
        high,
        ..Default::default()
    };
    let mut query = db.prepare("SELECT document_json,revision,index_body FROM telegram_mapped_documents WHERE space_id=? ORDER BY rowid")?;
    let mut rows = query.query([space])?;
    while let Some(row) = rows.next()? {
        let doc: RemoteDocument = serde_json::from_str(row.get_ref(0)?.as_str()?)?;
        if let Some(revision) = row.get::<_, Option<i64>>(1)? {
            snapshot.indexes.push(IndexDocument {
                revision,
                document: doc,
                source: IndexSource::Verified(row.get::<_, Vec<u8>>(2)?.into()),
            });
        } else {
            snapshot.media.push(doc);
        }
    }
    snapshot.order();
    Ok(snapshot)
}

fn save(
    db: &Connection,
    target: &Target,
    pts: i32,
    high: i64,
    changed: Option<&BTreeSet<i32>>,
    fresh: &ChannelSnapshot,
) -> Result<()> {
    let space = &target.scope.space_id;
    let tx = db.unchecked_transaction()?;
    tx.execute("INSERT INTO telegram_message_map(space_id,target_json,pts,high_water) VALUES(?1,?2,?3,?4)
        ON CONFLICT(space_id) DO UPDATE SET target_json=excluded.target_json,pts=excluded.pts,high_water=excluded.high_water",
        params![space,serde_json::to_string(target)?,pts,high])?;
    if let Some(ids) = changed {
        let mut remove = tx
            .prepare("DELETE FROM telegram_mapped_documents WHERE space_id=?1 AND message_id=?2")?;
        for id in ids {
            remove.execute(params![space, id])?;
        }
    } else {
        tx.execute(
            "DELETE FROM telegram_mapped_documents WHERE space_id=?",
            [space],
        )?;
    }
    {
        let mut put = tx.prepare("INSERT INTO telegram_mapped_documents(space_id,message_id,document_json,revision,index_body) VALUES(?1,?2,?3,?4,?5)")?;
        for index in &fresh.indexes {
            let IndexSource::Verified(body) = &index.source else {
                bail!("TELEGRAM_INDEX_INVALID");
            };
            put.execute(params![
                space,
                index.document.message_id,
                serde_json::to_string(&index.document)?,
                index.revision,
                body.as_slice()
            ])?;
        }
        for doc in &fresh.media {
            put.execute(params![
                space,
                doc.message_id,
                serde_json::to_string(doc)?,
                None::<i64>,
                None::<Vec<u8>>
            ])?;
        }
    }
    tx.commit()?;
    Ok(())
}

impl GrammersTransport {
    async fn changed_since(&self, pts: i32, io: &Io) -> Result<ChannelChanges> {
        io.run(async {
            let result = self
                .client
                .invoke(&tl::functions::updates::GetChannelDifference {
                    force: false,
                    channel: self.peer.into(),
                    filter: tl::enums::ChannelMessagesFilter::Empty,
                    pts,
                    limit: 100,
                })
                .await;
            match result {
                Ok(difference) => Ok(changed_messages(
                    difference,
                    self.peer.id.bare_id_unchecked(),
                    pts,
                )),
                Err(InvocationError::Rpc(e)) if e.name == "PERSISTENT_TIMESTAMP_EMPTY" => {
                    Ok(ChannelChanges::Unusable("PERSISTENT_TIMESTAMP_EMPTY"))
                }
                Err(InvocationError::Rpc(e)) if e.name == "PERSISTENT_TIMESTAMP_INVALID" => {
                    Ok(ChannelChanges::Unusable("PERSISTENT_TIMESTAMP_INVALID"))
                }
                Err(error) => Err(rpc_error(error)),
            }
        })
        .await
    }

    // Capture the position before a rebuild: edits made during pagination remain due next time.
    async fn current_pts(&self, io: &Io) -> Result<i32> {
        io.run(async {
            let response = self
                .client
                .invoke(&tl::functions::messages::GetHistory {
                    peer: self.peer.into(),
                    offset_id: 0,
                    offset_date: 0,
                    add_offset: 0,
                    limit: 1,
                    max_id: 0,
                    min_id: 0,
                    hash: 0,
                })
                .await
                .map_err(rpc_error)?;
            match response {
                tl::enums::messages::Messages::ChannelMessages(history) if history.pts > 0 => {
                    Ok(history.pts)
                }
                _ => bail!("TELEGRAM_INDEX_INVALID"),
            }
        })
        .await
    }

    pub(in crate::backup) async fn mapped_snapshot(
        &self,
        core: &Core,
        target: &Target,
        io: &Io,
    ) -> Result<ChannelSnapshot> {
        refresh(core, target, io, self).await
    }
}

trait MessageSource {
    async fn changes(&self, pts: i32, io: &Io) -> Result<ChannelChanges>;
    async fn full(&self, io: &Io) -> Result<(i32, ChannelSnapshot)>;
    async fn read_body(&self, index: &IndexDocument, io: &Io) -> Result<Vec<u8>>;
    async fn by_id(&self, ids: &BTreeSet<i32>, io: &Io) -> Result<ChannelSnapshot>;
}

impl MessageSource for GrammersTransport {
    async fn changes(&self, pts: i32, io: &Io) -> Result<ChannelChanges> {
        self.changed_since(pts, io).await
    }
    async fn full(&self, io: &Io) -> Result<(i32, ChannelSnapshot)> {
        let (pts, snapshot) = {
            let _history = delete_timing::refresh_span("history");
            (self.current_pts(io).await?, self.scan(io).await?)
        };
        Ok((pts, snapshot))
    }
    async fn read_body(&self, index: &IndexDocument, io: &Io) -> Result<Vec<u8>> {
        self.index_body(index, io).await
    }
    async fn by_id(&self, ids: &BTreeSet<i32>, io: &Io) -> Result<ChannelSnapshot> {
        let mut snapshot = ChannelSnapshot {
            high: 0,
            ..Default::default()
        };
        let requested: Vec<i32> = ids.iter().copied().collect();
        for batch in requested.chunks(100) {
            let _batch = delete_timing::refresh_span("by_id");
            let messages = io
                .run(async {
                    self.client
                        .get_messages_by_id(self.peer, batch)
                        .await
                        .map_err(rpc_error)
                })
                .await?;
            for message in messages.into_iter().flatten() {
                if !matches!(message.raw, tl::enums::Message::Empty(_)) {
                    snapshot.high = snapshot.high.max(i64::from(message.id()));
                    self.collect_message(message, io, &mut snapshot).await?;
                }
            }
        }
        verify_indexes(self, snapshot, io).await
    }
}

async fn verify_indexes(
    source: &impl MessageSource,
    mut snapshot: ChannelSnapshot,
    io: &Io,
) -> Result<ChannelSnapshot> {
    for index in &mut snapshot.indexes {
        io.check()?;
        if matches!(index.source, IndexSource::Scanned(_)) {
            index.source = IndexSource::Verified(source.read_body(index, io).await?.into());
        }
    }
    snapshot.order();
    Ok(snapshot)
}

// Only the bytes are reusable. Membership, revision and attachment identity come from the live scan.
fn reuse_index_bodies(
    core: &Core,
    target: &Target,
    snapshot: &mut ChannelSnapshot,
    io: &Io,
) -> Result<usize> {
    use sha2::{Digest, Sha256};
    let target_json = serde_json::to_string(target)?;
    let mut reused = 0;
    for index in &mut snapshot.indexes {
        io.check()?;
        let doc = &index.document;
        let body: Option<Vec<u8>> = core.backup_db(|db| {
            let mut query = db.prepare_cached(
                "SELECT d.index_body FROM telegram_mapped_documents d
                 JOIN telegram_message_map m USING(space_id)
                 WHERE d.space_id=?1 AND m.target_json=?2 AND d.message_id=?3
                   AND d.document_json=?4 AND d.revision IS NOT NULL AND length(d.index_body)=?5",
            )?;
            let key = params![
                target.scope.space_id,
                target_json,
                doc.message_id,
                serde_json::to_string(doc)?,
                i64::try_from(doc.bytes)?
            ];
            Ok(query.query_row(key, |row| row.get(0)).optional()?)
        })?;
        if let Some(body) = body {
            let envelope: Envelope =
                serde_json::from_str(&doc.caption).context("TELEGRAM_INDEX_INVALID")?;
            if envelope.bytes == doc.bytes
                && format!("{:x}", Sha256::digest(&body)) == envelope.sha256
            {
                index.source = IndexSource::Verified(body.into());
                reused += 1;
            }
        }
    }
    Ok(reused)
}

async fn refresh(
    core: &Core,
    target: &Target,
    io: &Io,
    source: &impl MessageSource,
) -> Result<ChannelSnapshot> {
    io.check()?;
    let space = &target.scope.space_id;
    let previous: Option<(i32, i64)> = core.backup_db(|db| {
        Ok(db.query_row(
        "SELECT pts,high_water FROM telegram_message_map WHERE space_id=?1 AND target_json=?2",
        params![space,serde_json::to_string(target)?], |r|Ok((r.get(0)?,r.get(1)?))).optional()?)
    })?;
    if let Some((old_pts, old_high)) = previous {
        let difference = {
            let _changes = delete_timing::refresh_span("channel_changes");
            source.changes(old_pts, io).await?
        };
        match difference {
            ChannelChanges::Delta(pts, ids) => {
                let fresh = if ids.is_empty() {
                    ChannelSnapshot::default()
                } else {
                    source.by_id(&ids, io).await?
                };
                let high = old_high.max(fresh.high);
                io.check()?;
                return core.backup_db(|db| {
                    if pts != old_pts {
                        save(db, target, pts, high, Some(&ids), &fresh)?;
                    }
                    load(db, space, high)
                });
            }
            ChannelChanges::Unusable(reason) => {
                delete_timing::selected_full_scan("difference_unusable", Some(reason))
            }
        }
    } else {
        delete_timing::selected_full_scan("missing_mapping", None);
    }
    let (pts, mut fresh) = source.full(io).await?;
    if previous.is_some() {
        let reused = reuse_index_bodies(core, target, &mut fresh, io)?;
        delete_timing::record_index_reuse(reused);
    }
    let fresh = verify_indexes(source, fresh, io).await?;
    io.check()?;
    core.backup_db(|db| save(db, target, pts, fresh.high, None, &fresh))?;
    Ok(fresh)
}

#[cfg(test)]
#[path = "telegram_message_map_tests.rs"]
mod tests;
