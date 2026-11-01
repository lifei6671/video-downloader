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
