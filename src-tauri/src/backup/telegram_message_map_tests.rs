use super::*;
use crate::backup::provider::{ProviderType, Scope};
use std::collections::VecDeque;
use std::sync::Mutex;

enum Step {
    Full(i32, ChannelSnapshot),
    Body(i32),
    Changes(i32, Option<(i32, Vec<i32>)>),
    Read(Vec<i32>, Result<ChannelSnapshot>),
}
struct Channel(Mutex<VecDeque<Step>>);
impl Channel {
    fn take(&self) -> Step {
        self.0
            .lock()
            .unwrap()
            .pop_front()
            .expect("unexpected network call")
    }
}
impl MessageSource for Channel {
    async fn full(&self, _: &Io) -> Result<(i32, ChannelSnapshot)> {
        let Step::Full(pts, snapshot) = self.take() else {
            panic!("unexpected history scan")
        };
        Ok((pts, snapshot))
    }
    async fn changes(&self, pts: i32, _: &Io) -> Result<ChannelChanges> {
        let Step::Changes(expected, result) = self.take() else {
            panic!("unexpected difference request")
        };
        assert_eq!(pts, expected);
        Ok(match result {
            Some((pts, ids)) => ChannelChanges::Delta(pts, ids.into_iter().collect()),
            None => ChannelChanges::Unusable("difference_too_long"),
        })
    }
    async fn read_body(&self, index: &IndexDocument, _: &Io) -> Result<Vec<u8>> {
        let Step::Body(expected) = self.take() else {
            panic!("unexpected index download")
        };
        assert_eq!(index.document.message_id, expected);
        assert!(matches!(index.source, IndexSource::Scanned(_)));
        Ok(b"{}".to_vec())
    }
    async fn by_id(&self, ids: &BTreeSet<i32>, _: &Io) -> Result<ChannelSnapshot> {
        let Step::Read(expected, result) = self.take() else {
            panic!("unexpected message fetch")
        };
        assert_eq!(ids.iter().copied().collect::<Vec<_>>(), expected);
        result
    }
}

fn doc(id: i32, kind: &str) -> RemoteDocument {
    let owner = uuid::Uuid::from_u128(1).to_string();
    let envelope = Envelope {
        yingqu_media: 2,
        library_id: owner.clone(),
        asset_id: owner.clone(),
        device_id: owner,
        operation_id: uuid::Uuid::from_u128(id as u128).to_string(),
        kind: kind.into(),
        bytes: 2,
        sha256: format!("{:x}", <sha2::Sha256 as sha2::Digest>::digest(b"{}")),
        container: "json".into(),
        generation: None,
    };
    RemoteDocument {
        message_id: id,
        document_id: i64::from(id) * 100,
        bytes: 2,
        caption: envelope.caption().unwrap(),
    }
}
fn image(library: i32, videos: &[i32]) -> ChannelSnapshot {
    ChannelSnapshot {
        high: i64::from(library),
        indexes: vec![IndexDocument {
            revision: i64::from(library),
            document: doc(library, "library"),
            source: IndexSource::Verified(b"{}".to_vec().into()),
        }],
        media: videos.iter().map(|id| doc(*id, "video")).collect(),
    }
}
#[tokio::test]
async fn mapped_delete_restart_only_reads_changed_ids_and_rebuilds_on_lost_position() {
    let root = tempfile::tempdir().unwrap();
    let core = open(root.path());
    let target = setup_target(&core);
    let (_stop, rx) = tokio::sync::watch::channel(false);
    let io = Io::new(rx.clone(), rx, None);
    // 300 old messages, a new archive and two removals. Unrelated messages must never be fetched.
    let vids: Vec<i32> = (2..=301).collect();
    let ids = vec![1, 42, 302];
    let channel = Channel(Mutex::new(VecDeque::from([
        Step::Full(12, image(1, &vids)),
        Step::Changes(12, Some((12, vec![]))),
        Step::Changes(12, Some((15, ids.clone()))),
        Step::Read(
            ids.clone(),
            Err(anyhow::anyhow!("TELEGRAM_CONNECTION_ERROR")),
        ),
        Step::Changes(12, Some((15, ids.clone()))),
        Step::Read(ids, Ok(image(302, &[]))),
        Step::Changes(15, None),
        Step::Full(40, image(405, &[70])),
        Step::Full(41, image(406, &[71])),
    ])));
    core.backup_db(|db| {
        db.execute("INSERT INTO backup_operations(operation_id,space_id,kind,state,phase,payload_json) VALUES('d','mapped','delete','running','objects','{}')", [])?;
        Ok(())
    }).unwrap();
    let first = delete_timing::IndexTiming::new(&core, "d", "archive")
        .refresh(refresh(&core, &target, &io, &channel))
        .await
        .unwrap();
    assert_eq!(
        details(&core, "archive")["full_scan_reason"],
        "missing_mapping"
    );
    assert_eq!(first.media.len(), 300);
    core.shutdown().await;
    drop(core);
    let core = open(root.path());
    let same = delete_timing::IndexTiming::new(&core, "d", "publish")
        .refresh(refresh(&core, &target, &io, &channel))
        .await
        .unwrap();
    assert_eq!(details(&core, "publish")["full_scan"], false);
    assert_eq!(
        details(&core, "publish")["steps"]["channel_changes"]["calls"],
        1
    );
    assert!(same
        .has_operation(&uuid::Uuid::from_u128(42).to_string())
        .unwrap());
    assert_eq!(
        refresh(&core, &target, &io, &channel)
            .await
            .err()
            .unwrap()
            .to_string(),
        "TELEGRAM_CONNECTION_ERROR"
    );
    let updated = refresh(&core, &target, &io, &channel).await.unwrap();
    assert_eq!(updated.media.len(), 299);
    assert!(!updated
        .has_operation(&uuid::Uuid::from_u128(42).to_string())
        .unwrap());
    assert_eq!(updated.indexes.len(), 1);
    assert_eq!(updated.indexes[0].document, doc(302, "library"));
    let IndexSource::Verified(bytes) = &updated.indexes[0].source else {
        panic!("missing archive body")
    };
    assert_eq!(bytes.as_slice(), b"{}");
    let rebuilt = delete_timing::IndexTiming::new(&core, "d", "archive")
        .refresh(refresh(&core, &target, &io, &channel))
        .await
        .unwrap();
    assert_eq!(
        details(&core, "archive")["full_scan_reason"],
        "difference_unusable"
    );
    assert_eq!(
        details(&core, "archive")["steps"]["channel_changes"]["calls"],
        1
    );
    assert_eq!(details(&core, "publish")["full_scan"], false);
    assert_eq!(
        details(&core, "archive")["difference_reason"],
        "difference_too_long"
    );
    assert_eq!(rebuilt.media, vec![doc(70, "video")]);
    let mut other = target.clone();
    other.channel_id = "3".into();
    assert_eq!(
        refresh(&core, &other, &io, &channel).await.unwrap().media,
        vec![doc(71, "video")]
    );
    assert!(channel.0.lock().unwrap().is_empty());
    core.shutdown().await;
}

#[test]
fn partial_channel_differences_never_advance_the_saved_position() {
    let difference = |complete, channel| {
        tl::enums::updates::ChannelDifference::Difference(tl::types::updates::ChannelDifference {
            r#final: complete,
            pts: 11,
            timeout: None,
            new_messages: vec![],
            other_updates: vec![tl::types::UpdateDeleteChannelMessages {
                channel_id: channel,
                messages: vec![5, 7, 5],
                pts: 11,
                pts_count: 1,
            }
            .into()],
            chats: vec![],
            users: vec![],
        })
    };
    assert_eq!(
        changed_messages(difference(true, 2), 2, 10),
        ChannelChanges::Delta(11, [5, 7].into_iter().collect())
    );
    assert_eq!(
        changed_messages(difference(false, 2), 2, 10),
        ChannelChanges::Unusable("difference_not_final")
    );
    assert_eq!(
        changed_messages(difference(true, 3), 2, 10),
        ChannelChanges::Unusable("delete_channel_mismatch")
    );
    assert_eq!(
        changed_messages(difference(true, 2), 2, 12),
        ChannelChanges::Unusable("pts_regressed")
    );
    assert_eq!(
        changed_messages(difference(true, 2), 2, 11),
        ChannelChanges::Unusable("changed_ids_without_pts_advance")
    );
    let mut with_receipts = difference(true, 2);
    let tl::enums::updates::ChannelDifference::Difference(d) = &mut with_receipts else {
        unreachable!()
    };
    for id in [7, 8, 8] {
        d.other_updates.push(
            tl::types::UpdateMessageId {
                id,
                random_id: i64::from(id) + 100,
            }
            .into(),
        );
    }
    assert_eq!(
        changed_messages(with_receipts.clone(), 2, 10),
        ChannelChanges::Delta(11, [5, 7, 8].into_iter().collect())
    );
    assert_eq!(
        changed_messages(with_receipts.clone(), 2, 11),
        ChannelChanges::Unusable("changed_ids_without_pts_advance")
    );
    let tl::enums::updates::ChannelDifference::Difference(d) = &mut with_receipts else {
        unreachable!()
    };
    d.other_updates.push(tl::enums::Update::PtsChanged);
    assert_eq!(
        changed_messages(with_receipts, 2, 10),
        ChannelChanges::Unusable("updatePtsChanged")
    );
}

fn scanned(docs: Vec<RemoteDocument>) -> ChannelSnapshot {
    let indexes = docs
        .into_iter()
        .map(|document| {
            let attachment = tl::types::Document {
                id: document.document_id,
                access_hash: 0,
                file_reference: vec![],
                date: 0,
                mime_type: "application/json".into(),
                size: document.bytes as i64,
                thumbs: None,
                video_thumbs: None,
                dc_id: 1,
                attributes: vec![],
            };
            IndexDocument {
                revision: i64::from(document.message_id),
                document,
                source: IndexSource::Scanned(attachment.into()),
            }
        })
        .collect();
    ChannelSnapshot {
        indexes,
        ..Default::default()
    }
}

#[tokio::test]
async fn full_refresh_reuses_only_verified_matching_bodies_from_this_target() {
    let root = tempfile::tempdir().unwrap();
    let core = open(root.path());
    let target = setup_target(&core);
    let (_stop, rx) = tokio::sync::watch::channel(false);
    let io = Io::new(rx.clone(), rx, None);

    core.backup_db(|db| {
        db.execute("INSERT INTO backup_operations(operation_id,space_id,kind,state,phase,payload_json) VALUES('d','mapped','delete','running','objects','{}')", [])?;
        Ok(())
    }).unwrap();
    let cold: Vec<_> = (1..=5).map(|id| doc(id, "library")).collect();
    let mut current = cold[..4].to_vec();
    current[1].document_id += 1; // Attachment replaced under the same message and caption.
    current[2].caption = doc(30, "library").caption; // Same attachment, edited envelope.
    current.push(doc(6, "library")); // 5 removed, 6 newly published.
    let channel = Channel(Mutex::new(VecDeque::from([
        Step::Full(10, scanned(cold)),
        Step::Body(1),
        Step::Body(2),
        Step::Body(3),
        Step::Body(4),
        Step::Body(5),
        Step::Changes(10, None),
        Step::Full(20, scanned(current.clone())),
        Step::Body(2),
        Step::Body(3),
        Step::Body(4),
        Step::Body(6),
        Step::Full(30, scanned(vec![current[0].clone()])),
        Step::Body(1),
    ])));
    refresh(&core, &target, &io, &channel).await.unwrap();
    core.backup_db(|db| {
        db.execute("UPDATE telegram_mapped_documents SET index_body=x'3030' WHERE space_id='mapped' AND message_id=4", [])?;
        Ok(())
    }).unwrap();
    let timing = delete_timing::IndexTiming::new(&core, "d", "publish");
    let fresh = timing
        .refresh(refresh(&core, &target, &io, &channel))
        .await
        .unwrap();
    assert_eq!(details(&core, "publish")["reused_index_bodies"], 1);
    assert_eq!(
        fresh
            .indexes
            .iter()
            .map(|i| &i.document)
            .collect::<Vec<_>>(),
        current.iter().collect::<Vec<_>>()
    );
    core.backup_db(|db| {
        let saved = load(db, "mapped", 0)?;
        assert_eq!(saved.indexes.len(), current.len());
        for (index, expected) in saved.indexes.iter().zip(&current) {
            assert_eq!(&index.document, expected);
            assert!(matches!(&index.source, IndexSource::Verified(body) if body.as_slice()==b"{}"));
        }
        Ok(())
    })
    .unwrap();
    let mut other = target.clone();
    other.channel_id = "3".into();
    timing
        .refresh(refresh(&core, &other, &io, &channel))
        .await
        .unwrap();
    assert_eq!(details(&core, "publish")["reused_index_bodies"], 0);
    assert!(details(&core, "publish")["difference_reason"].is_null());
    assert!(channel.0.lock().unwrap().is_empty());
    core.shutdown().await;
}
fn details(core: &Core, stage: &str) -> serde_json::Value {
    core.backup_db(|db| {
        let raw: String = db.query_row(
            "SELECT json_extract(payload_json,?1) FROM backup_operations WHERE operation_id='d'",
            [format!("$.timings.foreground.{stage}_refresh.details")],
            |row| row.get(0),
        )?;
        Ok(serde_json::from_str(&raw)?)
    })
    .unwrap()
}

fn open(root: &std::path::Path) -> std::sync::Arc<Core> {
    Core::open(
        &root.join("qa.sqlite"),
        root.join("downloads"),
        "unused".into(),
        vec![],
        |_| {},
    )
    .unwrap()
}
fn setup_target(core: &Core) -> Target {
    let target = Target {
        account_id: "1".into(),
        channel_id: "2".into(),
        scope: Scope {
            space_id: "mapped".into(),
            library_id: uuid::Uuid::from_u128(1).to_string(),
            protocol: 2,
            provider_type: ProviderType::Telegram,
        },
    };
    core.backup_db(|db|{db.execute("INSERT INTO backup_spaces(space_id,library_id,endpoint,bucket,prefix,enabled) VALUES('mapped',?1,'','','',0)",[&target.scope.library_id])?;Ok(())}).unwrap();
    target
}

#[tokio::test]
async fn direct_delete_archive_survives_lost_reply_upgrade_delta_and_fresh_rebuild() -> Result<()> {
    use crate::backup::{
        provider::{AssetChange, Checkpoint, MediaLocation, MediaRef, ReferenceChange},
        telegram_provider::TelegramProvider as Provider, telegram_replay::{Cursor, Event},
        telegram_transfer::tests::send_archive_losing_reply,
        telegram_transfer_journal::{CoreJournal, Journal, SendState},
    };
    use sha2::{Digest, Sha256};
    fn index(id: i32, kind: &str, asset: &str, operation: &str, device: &str, body: Vec<u8>) -> IndexDocument {
        let mut document = doc(id, kind);
        let mut envelope: Envelope = serde_json::from_str(&document.caption).unwrap();
        envelope.asset_id = asset.into();
        envelope.operation_id = operation.into();
        envelope.device_id = device.into();
        envelope.sha256 = format!("{:x}", Sha256::digest(&body));
        envelope.bytes = body.len() as u64;
        document.bytes = envelope.bytes;
        document.caption = envelope.caption().unwrap();
        IndexDocument { revision: if kind == "event" { (i64::from(id)<<32)|1 } else { i64::from(id) },
            document, source: IndexSource::Verified(body.into()) }
    }
    fn snapshot(indexes: Vec<IndexDocument>) -> ChannelSnapshot {
        let mut snapshot = ChannelSnapshot { indexes, ..Default::default() };
        snapshot.order();
        snapshot
    }
    let root = tempfile::tempdir()?;
    let core = open(root.path());
    let target = setup_target(&core);
    let library = &target.scope.library_id;
    let device = uuid::Uuid::new_v4().to_string();
    let asset = uuid::Uuid::new_v4().to_string();
    let (_stop, rx) = tokio::sync::watch::channel(false);
    let io = Io::new(rx.clone(), rx, None);
    let create = AssetChange { recreate:false,asset_id:asset.clone(),base:None,
        fields: [("original_filename".into(),serde_json::json!("fixture.mp4"))].into(),
        media:Some(MediaRef { scope:target.scope.clone(), location:MediaLocation::TelegramMessage {
            account_id:target.account_id.clone(), channel_id:target.channel_id.clone(), message_id:10, document_id:100 },
            bytes:32,sha256:"ab".repeat(32),container:"mp4".into()}),
        cover:ReferenceChange::Unchanged,transcript:ReferenceChange::Unchanged,delete:false,
        operation_id:uuid::Uuid::new_v4().to_string(),device_id:device.clone(),
        conflicts:vec![],resolved_conflicts:vec![],field_bases:Default::default() };
    let old_library = serde_json::to_vec(&serde_json::json!({"schema_version":2,"library_id":library,
        "provider_type":"telegram","mode":"single_writer","account_id":target.account_id,"channel_id":target.channel_id}))?;
    let initial = snapshot(vec![index(5,"library",library,library,&device,old_library),
        index(10,"event",&asset,&create.operation_id,&device,Event::encode(&target,&create)?)]);
    let old = refresh(&core,&target,&io,&Channel(Mutex::new([Step::Full(50,initial)].into()))).await?;
    let before = Provider::project_snapshot(&core,&target,&io,None,&old)?;
    let checkpoint = Checkpoint {scope:target.scope.clone(),value:serde_json::to_string(&before.cursor)?};
    core.backup_db(|db| {
        db.execute("INSERT INTO backup_sync_checkpoints(space_id,checkpoint_json) VALUES(?1,?2)",
            params![target.scope.space_id,serde_json::to_string(&checkpoint)?])?;
        Ok(())
    })?;
    let change = AssetChange { delete:true, media:None, fields:Default::default(), base:Some(before.assets[&asset].clone()),
        operation_id:uuid::Uuid::new_v4().to_string(), ..create };
    let event = Event::encode(&target,&change)?;
    let body = Provider::direct_archive_body(&io,&target,&asset,&old,event.clone())?;
    let (carrier, committed_bytes) = send_archive_losing_reply(root.path(),&CoreJournal(&core),&target,
        &change.operation_id,&device,&body,&io).await;
    assert_eq!(body,committed_bytes);
    let uncertain = CoreJournal(&core).get(&change.operation_id,"library")?.unwrap();
    assert_eq!(Provider::prepare_archive_retry(&core,&target,&change,b"changed library history").unwrap_err().to_string(),
        "TELEGRAM_SEND_UNCONFIRMED");
    assert_eq!(CoreJournal(&core).get(&change.operation_id,"library")?.unwrap().random_id,uncertain.random_id);
    // A different operation that did not reach send may safely rebuild its draft after history changes.
    let mut waiting = change.clone();
    waiting.operation_id = uuid::Uuid::new_v4().to_string();
    let mut prepared = uncertain.clone();
    prepared.envelope.operation_id.clone_from(&waiting.operation_id);
    prepared.state = SendState::Prepared;
    prepared.last_error = Some("TELEGRAM_FLOOD_WAIT:2".into());
    CoreJournal(&core).reserve(prepared)?;
    Provider::prepare_archive_retry(&core,&target,&waiting,&body)?;
    assert!(CoreJournal(&core).get(&waiting.operation_id,"library")?.is_some());
    Provider::prepare_archive_retry(&core,&target,&waiting,b"changed library history")?;
    assert!(CoreJournal(&core).get(&waiting.operation_id,"library")?.is_none());
    assert_eq!(CoreJournal(&core).get(&change.operation_id,"library")?.unwrap().state,SendState::Sending);
    core.shutdown().await;
    drop(core);

    // Restart with the old mapping/checkpoint. Only the newly added archive ID is requested.
    let core = open(root.path());
    let raw:String = core.backup_db(|db| Ok(db.query_row("SELECT checkpoint_json FROM backup_sync_checkpoints WHERE space_id=?",
        [&target.scope.space_id],|r| r.get(0))?))?;
    let cursor:Cursor = serde_json::from_str(&serde_json::from_str::<Checkpoint>(&raw)?.value)?;
    let direct = IndexDocument { revision:30, document:carrier.clone(), source:IndexSource::Verified(committed_bytes.into()) };
    let delta = Channel(Mutex::new([Step::Changes(50,Some((51,vec![30]))),
        Step::Read(vec![30],Ok(snapshot(vec![direct.clone()])))].into()));
    let both = refresh(&core,&target,&io,&delta).await?;
    let applied = Provider::project_snapshot(&core,&target,&io,Some(&cursor),&both)?;
    assert!(Provider::publication_exists(&CoreJournal(&core),&target,&change,&event,&applied,&both)?);
    let receipt = CoreJournal(&core).get(&change.operation_id,"library")?.unwrap();
    assert_eq!(receipt.state,SendState::Confirmed);
    assert_eq!(receipt.document,Some(carrier));
    let revision = (30_u64<<32)|1;
    assert!(applied.assets[&asset].deleted);
    assert_eq!(applied.assets[&asset].revision,revision);
    let removals = Channel(Mutex::new([Step::Changes(51,Some((52,vec![5,10]))),
        Step::Read(vec![5,10],Ok(snapshot(vec![])))].into()));
    let only = refresh(&core,&target,&io,&removals).await?;
    assert_eq!(only.indexes.len(),1);
    let final_state = Provider::project_snapshot(&core,&target,&io,Some(&applied.cursor),&only)?;
    assert_eq!(applied.cursor.evidence,final_state.cursor.evidence);

    // A new SQLite database with no mapping or cursor sees only the surviving library.
    let fresh_root = tempfile::tempdir()?;
    let fresh = open(fresh_root.path());
    let mut fresh_target = setup_target(&fresh);
    fresh_target.scope.library_id = uuid::Uuid::new_v4().to_string();
    let envelope: Envelope = serde_json::from_str(&direct.document.caption)?;
    let discovered = Provider::descriptor_library_id(&fresh_target,&envelope,direct.verified_body()?)?;
    assert_eq!(discovered,target.scope.library_id);
    fresh_target.scope.library_id = discovered;
    let scan = Channel(Mutex::new([Step::Full(52,snapshot(vec![direct]))].into()));
    let only = refresh(&fresh,&fresh_target,&io,&scan).await?;
    let restored = Provider::project_snapshot(&fresh,&fresh_target,&io,None,&only)?;
    assert!(restored.assets[&asset].deleted);
    assert_eq!(restored.assets[&asset].revision,revision);
    assert_eq!(restored.cursor.evidence,applied.cursor.evidence);

    // A later archive freezes the first deletion at the original carrier revision.
    let second = AssetChange {asset_id:uuid::Uuid::new_v4().to_string(),operation_id:uuid::Uuid::new_v4().to_string(),base:None,..change};
    let next = Provider::direct_archive_body(&io,&target,&second.asset_id,&only,Event::encode(&target,&second)?)?;
    let pair: crate::backup::telegram_provider::Library = serde_json::from_slice(&next)?;
    let both = [(asset.as_str(),revision),(second.asset_id.as_str(),(39_u64<<32)|1)];
    pair.confirm_deletions(&target,39,&both)?;
    assert!(pair.confirm_deletions(&target,39,&[(asset.as_str(),revision+1)]).is_err());
    let next = snapshot(vec![index(39,"library",library,&second.operation_id,&device,next)]);
    let rolled = Provider::project_snapshot(&fresh,&fresh_target,&io,Some(&restored.cursor),&next)?;
    assert_eq!(rolled.assets[&asset].revision,revision);
    assert_eq!(rolled.assets[&second.asset_id].revision,(39_u64<<32)|1);
    core.shutdown().await;
    fresh.shutdown().await;
    Ok(())
}
