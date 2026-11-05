use super::*;
use crate::backup::provider::{ProviderType, Scope};
use std::collections::VecDeque;
use std::sync::Mutex;

enum Step {
    Full(i32, ChannelSnapshot),
    Body(i32),
    Changes(i32, Option<(i32, Vec<i32>)>),
    Page(i32, Result<DifferencePage>),
    Read(Vec<i32>, Result<ChannelSnapshot>),
}
struct Channel(Mutex<VecDeque<Step>>);
struct HeldChannel {
    inner: Channel,
    started: tokio::sync::Notify,
    release: tokio::sync::Semaphore,
}
impl HeldChannel {
    async fn wait(&self, io: &Io) -> Result<()> {
        self.started.notify_one();
        io.run(async { self.release.acquire().await?.forget(); Ok(()) }).await
    }
}
impl MessageSource for HeldChannel {
    async fn full(&self, io: &Io) -> Result<(i32, ChannelSnapshot)> {
        self.wait(io).await?;
        self.inner.full(io).await
    }
    async fn changes(&self, pts: i32, io: &Io) -> Result<DifferencePage> {
        self.wait(io).await?;
        self.inner.changes(pts, io).await
    }
    async fn read_body(&self, index: &IndexDocument, io: &Io) -> Result<Vec<u8>> { self.inner.read_body(index, io).await }
    async fn by_id(&self, ids: &BTreeSet<i32>, io: &Io) -> Result<ChannelSnapshot> { self.inner.by_id(ids, io).await }
}
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
    async fn changes(&self, pts: i32, _: &Io) -> Result<DifferencePage> {
        match self.take() {
            Step::Changes(expected, result) => {
                assert_eq!(pts, expected);
                Ok(match result {
                    Some((pts, ids)) => DifferencePage::Changes {
                        pts,
                        ids: ids.into_iter().collect(),
                        deleted: BTreeSet::new(),
                        complete: true,
                    },
                    None => DifferencePage::Unusable("difference_too_long"),
                })
            }
            Step::Page(expected, result) => {
                assert_eq!(pts, expected);
                result
            }
            _ => panic!("unexpected difference request"),
        }
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
async fn concurrent_full_and_incremental_refreshes_admit_before_reading_the_cursor() {
    for incremental in [false, true] {
        let root = tempfile::tempdir().unwrap();
        let core = open(root.path());
        let target = setup_target(&core);
        let (_stop, rx) = tokio::sync::watch::channel(false);
        let io = Io::new(rx.clone(), rx, None);
        if incremental {
            let seed = Channel(Mutex::new(VecDeque::from([Step::Full(12, image(1, &[2]))])));
            refresh(&core, &target, &io, &seed).await.unwrap();
        }
        let older = HeldChannel {
            inner: Channel(Mutex::new(if incremental {
                VecDeque::from([Step::Changes(12, Some((15, vec![1, 3]))), Step::Read(vec![1, 3], Ok(image(3, &[])))])
            } else { VecDeque::from([Step::Full(15, image(3, &[2]))]) })),
            started: tokio::sync::Notify::new(),
            release: tokio::sync::Semaphore::new(0),
        };
        let newer = Channel(Mutex::new(VecDeque::from([
            Step::Changes(15, Some((20, vec![2, 3, 4, 5]))),
            Step::Read(vec![2, 3, 4, 5], Ok(image(4, &[5]))),
        ])));
        let mut first = Box::pin(refresh(&core, &target, &io, &older));
        tokio::time::timeout(std::time::Duration::from_secs(1), async {
            tokio::select! { _ = older.started.notified() => {}, _ = &mut first => panic!("old scan ended early") }
        }).await.unwrap();
        let mut second = Box::pin(refresh(&core, &target, &io, &newer));
        assert!(tokio::time::timeout(std::time::Duration::from_millis(30), &mut second).await.is_err());
        assert_eq!(newer.0.lock().unwrap().len(), 2, "waiting refresh read a stale cursor or started network work");
        older.release.add_permits(1);
        let (first, second) = tokio::join!(first, second);
        first.unwrap();
        assert_eq!(second.unwrap().media.iter().map(|d| d.message_id).collect::<Vec<_>>(), vec![5]);
        core.backup_db(|db| {
            let cursor: (i32, i64) = db.query_row("SELECT pts,high_water FROM telegram_message_map WHERE space_id=?", [&target.scope.space_id], |r| Ok((r.get(0)?,r.get(1)?)))?;
            assert_eq!(cursor, (20, 4));
            assert_eq!(load(db, &target.scope.space_id, cursor.1)?.media.iter().map(|d| d.message_id).collect::<Vec<_>>(), vec![5]);
            Ok(())
        }).unwrap();
        core.shutdown().await;
    }
}

#[tokio::test]
async fn canceling_a_waiting_map_refresh_does_not_cancel_the_active_scan() {
    let root = tempfile::tempdir().unwrap();
    let core = open(root.path());
    let target = setup_target(&core);
    let (_login, login) = tokio::sync::watch::channel(false);
    let (stop, cancel) = tokio::sync::watch::channel(false);
    let first_io = Io::new(login.clone(), login.clone(), None);
    let second_io = Io::new(cancel, login, None);
    let older = HeldChannel { inner: Channel(Mutex::new(VecDeque::from([Step::Full(15, image(3, &[2]))]))), started: tokio::sync::Notify::new(), release: tokio::sync::Semaphore::new(0) };
    let newer = Channel(Mutex::new(VecDeque::new()));
    let mut first = Box::pin(refresh(&core, &target, &first_io, &older));
    tokio::select! { _ = older.started.notified() => {}, _ = &mut first => panic!("scan ended early") }
    let mut second = Box::pin(refresh(&core, &target, &second_io, &newer));
    assert!(tokio::time::timeout(std::time::Duration::from_millis(30), &mut second).await.is_err());
    stop.send_replace(true);
    assert_eq!(tokio::time::timeout(std::time::Duration::from_secs(1), &mut second).await.unwrap().err().unwrap().to_string(), "TELEGRAM_TRANSFER_CANCELED");
    older.release.add_permits(1);
    assert_eq!(first.await.unwrap().high, 3);
    assert!(core.telegram_map_refresh.try_lock().is_ok());
    core.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_slow_projection_observation_finishes_before_the_next_refresh() {
    use crate::backup::{telegram_provider::TelegramProvider, telegram_replay::Cursor};
    let root = tempfile::tempdir().unwrap();
    let core = open(root.path());
    let target = setup_target(&core);
    let (_login, login) = tokio::sync::watch::channel(false);
    let (started, entered) = tokio::sync::oneshot::channel();
    let (release, held) = std::sync::mpsc::channel();
    let first_core = core.clone();
    let first_target = target.clone();
    let first_login = login.clone();
    let first = tokio::spawn(async move {
        let io = Io::new(first_login.clone(), first_login, None);
        let channel = Channel(Mutex::new(VecDeque::from([Step::Full(12, image(1, &[]))])));
        let observe_core = first_core.clone();
        let observe_space = first_target.scope.space_id.clone();
        refresh_with(&first_core, &first_target, &io, &channel, move |_| {
            let _ = started.send(());
            // Model CPU projection on another runtime worker. Bound the gate so
            // even a failing assertion cannot leave a blocked test thread.
            held.recv_timeout(std::time::Duration::from_secs(3))?;
            TelegramProvider::observe(&observe_core, &observe_space,
                &Cursor {high_water:1,evidence:std::collections::BTreeMap::from([(1,"old".into())])})
        }).await
    });
    tokio::time::timeout(std::time::Duration::from_secs(1), entered).await.unwrap().unwrap();
    let io = Io::new(login.clone(), login, None);
    let channel = Channel(Mutex::new(VecDeque::from([
        Step::Changes(12, Some((15, vec![1, 2]))), Step::Read(vec![1, 2], Ok(image(2, &[]))),
    ])));
    let mut second = Box::pin(refresh_with(&core, &target, &io, &channel, |_| {
        TelegramProvider::observe(&core, &target.scope.space_id,
            &Cursor {high_water:2,evidence:std::collections::BTreeMap::from([(1,"old".into()),(2,"new".into())])})
    }));
    assert!(tokio::time::timeout(std::time::Duration::from_millis(30), &mut second).await.is_err());
    assert_eq!(channel.0.lock().unwrap().len(), 2, "new observation overtook a slow old projection");
    release.send(()).unwrap();
    first.await.unwrap().unwrap();
    second.await.unwrap();
    let saved: String = core.backup_db(|db| Ok(db.query_row("SELECT cursor_json FROM telegram_observed_prefix WHERE space_id=?", [&target.scope.space_id], |r| r.get(0))?)).unwrap();
    let saved: Cursor = serde_json::from_str(&saved).unwrap();
    assert_eq!(saved.high_water, 2);
    assert_eq!(saved.evidence.len(), 2);
    core.shutdown().await;
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
fn channel_difference_marks_pages_and_rejects_unusable_updates() {
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
        DifferencePage::Changes {
            pts: 11,
            ids: BTreeSet::new(),
            deleted: [5, 7].into_iter().collect(),
            complete: true
        }
    );
    assert_eq!(
        changed_messages(difference(false, 2), 2, 10),
        DifferencePage::Changes {
            pts: 11,
            ids: BTreeSet::new(),
            deleted: [5, 7].into_iter().collect(),
            complete: false
        }
    );
    assert_eq!(
        changed_messages(difference(true, 3), 2, 10),
        DifferencePage::Unusable("delete_channel_mismatch")
    );
    assert_eq!(
        changed_messages(difference(true, 2), 2, 12),
        DifferencePage::Unusable("pts_regressed")
    );
    assert_eq!(
        changed_messages(difference(true, 2), 2, 11),
        DifferencePage::Unusable("changed_ids_without_pts_advance")
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
        DifferencePage::Changes {
            pts: 11,
            ids: [7, 8].into_iter().collect(),
            deleted: [5, 7].into_iter().collect(),
            complete: true
        }
    );
    assert_eq!(
        changed_messages(with_receipts.clone(), 2, 11),
        DifferencePage::Unusable("changed_ids_without_pts_advance")
    );
    let tl::enums::updates::ChannelDifference::Difference(d) = &mut with_receipts else {
        unreachable!()
    };
    d.other_updates.push(tl::enums::Update::PtsChanged);
    assert_eq!(
        changed_messages(with_receipts, 2, 10),
        DifferencePage::Unusable("updatePtsChanged")
    );
}

#[test]
fn difference_requires_usable_pts_and_positive_channel_message_ids() {
    let empty = |complete, pts| {
        tl::types::updates::ChannelDifferenceEmpty {
            r#final: complete,
            pts,
            timeout: None,
        }
        .into()
    };
    assert_eq!(
        changed_messages(empty(false, 9), 2, 9),
        DifferencePage::Unusable("non_final_pts_stalled")
    );
    assert_eq!(
        changed_messages(empty(false, 10), 2, 9),
        DifferencePage::Changes {
            pts: 10,
            ids: BTreeSet::new(),
            deleted: BTreeSet::new(),
            complete: false
        }
    );
    assert_eq!(
        changed_messages(empty(true, 9), 2, 9),
        DifferencePage::Changes {
            pts: 9,
            ids: BTreeSet::new(),
            deleted: BTreeSet::new(),
            complete: true
        }
    );
    let peer = |channel_id| Some(tl::types::PeerChannel { channel_id }.into());
    let message = |id, peer_id| tl::types::MessageEmpty { id, peer_id }.into();
    let page = |new_messages, other_updates| {
        tl::types::updates::ChannelDifference {
            r#final: true,
            pts: 10,
            timeout: None,
            new_messages,
            other_updates,
            chats: vec![],
            users: vec![],
        }
        .into()
    };
    assert_eq!(
        changed_messages(page(vec![message(1, peer(3))], vec![]), 2, 9),
        DifferencePage::Unusable("message_channel_mismatch")
    );
    assert_eq!(
        changed_messages(
            page(
                vec![],
                vec![tl::types::UpdateEditChannelMessage {
                    message: message(1, peer(3)),
                    pts: 10,
                    pts_count: 1
                }
                .into()]
            ),
            2,
            9
        ),
        DifferencePage::Unusable("edit_channel_mismatch")
    );
    assert_eq!(
        changed_messages(
            page(
                vec![],
                vec![tl::types::UpdateNewChannelMessage {
                    message: message(1, peer(3)),
                    pts: 10,
                    pts_count: 1
                }
                .into()]
            ),
            2,
            9
        ),
        DifferencePage::Unusable("new_channel_mismatch")
    );
    assert_eq!(
        changed_messages(page(vec![message(0, peer(2))], vec![]), 2, 9),
        DifferencePage::Unusable("invalid_message_id")
    );
}

fn page(expected: i32, pts: i32, ids: impl IntoIterator<Item = i32>, complete: bool) -> Step {
    Step::Page(
        expected,
        Ok(DifferencePage::Changes {
            pts,
            ids: ids.into_iter().collect(),
            deleted: BTreeSet::new(),
            complete,
        }),
    )
}

#[tokio::test]
async fn paginated_channel_refresh_retries_from_saved_pts_and_reads_each_changed_id_once(
) -> Result<()> {
    let root = tempfile::tempdir()?;
    let core = open(root.path());
    let target = setup_target(&core);
    let (_stop, rx) = tokio::sync::watch::channel(false);
    let io = Io::new(rx.clone(), rx, None);
    let first = || Step::Page(12, Ok(DifferencePage::Changes {
        pts: 14, ids: BTreeSet::new(), deleted: [42, 43].into(), complete: false,
    }));
    let second = || page(14, 15, [43, 44], false);
    let channel = Channel(Mutex::new(VecDeque::from([
        Step::Full(12, image(1, &[42, 43, 44, 45])),
        first(),
        Step::Page(14, Err(anyhow::anyhow!("TELEGRAM_CONNECTION_ERROR"))),
        first(),
        second(),
        page(15, 15, [], true),
        Step::Read(vec![43, 44], Err(anyhow::anyhow!("CANCELED"))),
        first(),
        second(),
        page(15, 15, [], true),
        Step::Read(vec![43, 44], Ok(ChannelSnapshot::default())),
        Step::Page(15, Ok(DifferencePage::Changes {
            pts: 16, ids: BTreeSet::new(), deleted: [45].into(), complete: true,
        })),
    ])));
    refresh(&core, &target, &io, &channel).await?;
    for expected in ["TELEGRAM_CONNECTION_ERROR", "CANCELED"] {
        assert_eq!(
            refresh(&core, &target, &io, &channel)
                .await
                .err()
                .unwrap()
                .to_string(),
            expected
        );
        assert_eq!(
            core.backup_db(|db| load(db, &target.scope.space_id, 1))?
                .media
                .len(),
            4
        );
    }
    assert_eq!(refresh(&core, &target, &io, &channel).await?.media, vec![doc(45, "video")]);
    assert!(refresh(&core, &target, &io, &channel).await?.media.is_empty());
    assert!(channel.0.lock().unwrap().is_empty());
    core.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn paginated_refresh_falls_back_on_invalid_or_excessive_pages() -> Result<()> {
    let root = tempfile::tempdir()?;
    let core = open(root.path());
    let target = setup_target(&core);
    let (_stop, rx) = tokio::sync::watch::channel(false);
    let io = Io::new(rx.clone(), rx, None);
    core.backup_db(|db| {
        db.execute("INSERT INTO backup_operations(operation_id,space_id,kind,state,phase,payload_json) VALUES('d','mapped','delete','running','objects','{}')", [])?;
        Ok(())
    })?;
    let mut steps = VecDeque::from([
        Step::Full(10, image(1, &[2])),
        page(10, 11, [2], false),
        Step::Page(11, Ok(DifferencePage::Unusable("difference_too_long"))),
        Step::Full(20, image(2, &[3])),
        page(20, 21, 1..=800, false),
        page(21, 22, 801..=1601, true),
        Step::Full(30, image(3, &[4])),
    ]);
    for n in 0..MAX_DIFFERENCE_PAGES as i32 {
        steps.push_back(page(30 + n, 31 + n, [4], false));
    }
    steps.push_back(Step::Full(50, image(4, &[5])));
    // The last allowed page may be final, and IDs repeated across pages count once.
    for n in 0..MAX_DIFFERENCE_PAGES as i32 {
        steps.push_back(page(
            50 + n,
            51 + n,
            1..=1600,
            n + 1 == MAX_DIFFERENCE_PAGES as i32,
        ));
    }
    steps.push_back(Step::Read(
        (1..=1600).collect(),
        Ok(ChannelSnapshot::default()),
    ));
    let channel = Channel(Mutex::new(steps));
    refresh(&core, &target, &io, &channel).await?;
    let timing = delete_timing::IndexTiming::new(&core, "d", "archive");
    for (stage, reason, video, pages, unique_ids) in [
        ("later_too_long", "difference_too_long", 3, 2, 1),
        ("too_many_ids", "difference_id_limit", 4, 2, 1601),
        ("too_many_pages", "difference_page_limit", 5, 16, 1),
    ] {
        assert_eq!(
            delete_timing::IndexTiming::new(&core,"d",stage)
                .refresh(refresh(&core, &target, &io, &channel))
                .await?
                .media,
            vec![doc(video, "video")]
        );
        let detail = details(&core, stage);
        assert_eq!(detail["difference_reason"], reason);
        assert_eq!(detail["difference_pages"], pages);
        assert_eq!(detail["difference_changed_ids"], unique_ids);
    }
    assert!(timing
        .refresh(refresh(&core, &target, &io, &channel))
        .await?
        .media
        .is_empty());
    let detail = details(&core, "archive");
    assert_eq!(detail["full_scan"], false);
    assert_eq!(detail["steps"]["channel_changes"]["calls"], 16);
    assert_eq!(detail["difference_pages"], 16);
    assert_eq!(detail["difference_changed_ids"], 1600);
    assert!(channel.0.lock().unwrap().is_empty());
    core.shutdown().await;
    Ok(())
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
    // This is a new attempt against a different channel. Same-attempt metrics accumulate.
    core.backup_db(|db| {
        db.execute("UPDATE backup_operations SET attempt=attempt+1 WHERE operation_id='d'", [])?;
        Ok(())
    }).unwrap();
    timing
        .refresh(refresh(&core, &other, &io, &channel))
        .await
        .unwrap();
    assert_eq!(details(&core, "publish")["reused_index_bodies"], 0);
    assert!(details(&core, "publish")["difference_reason"].is_null());
    assert!(channel.0.lock().unwrap().is_empty());
    core.shutdown().await;
}
#[tokio::test]
async fn incremental_edit_reuses_only_unchanged_verified_attachments() -> Result<()> {
    let root = tempfile::tempdir()?;
    let core = open(root.path());
    let target = setup_target(&core);
    let (_stop, rx) = tokio::sync::watch::channel(false);
    let io = Io::new(rx.clone(), rx, None);
    let first = doc(10, "event");
    let mut second = doc(11, "event");
    second.message_id = first.message_id;
    let mut replaced = second.clone();
    replaced.document_id += 1;
    let original = scanned(vec![first.clone(), second]);
    let mut edited = scanned(vec![first.clone(), replaced.clone()]);
    edited.indexes[0].revision += 2; // Membership and revision must come from the live edit.
    let channel = Channel(Mutex::new(VecDeque::from([
        Step::Full(1, original), Step::Body(10), Step::Body(10),
        Step::Changes(1, Some((2, vec![10]))), Step::Read(vec![10], Ok(edited)),
        Step::Body(10), // Only the replacement requires a download.
    ])));
    refresh(&core, &target, &io, &channel).await?;
    let fresh = refresh(&core, &target, &io, &channel).await?;
    assert_eq!(fresh.indexes.len(), 2);
    assert_eq!(fresh.indexes[0].document, replaced);
    assert_eq!(fresh.indexes[1].document, first);
    assert_eq!(fresh.indexes[1].revision, 12);
    assert!(channel.0.lock().unwrap().is_empty());
    core.shutdown().await;
    Ok(())
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
    exercise_archive_upgrade(false).await
}

#[tokio::test]
async fn segmented_delete_rotates_legacy_archive_recovers_lost_reply_and_rebuilds() -> Result<()> {
    exercise_archive_upgrade(true).await
}

fn index(id: i32, kind: &str, asset: &str, operation: &str, device: &str, body: Vec<u8>) -> IndexDocument {
    use sha2::{Digest, Sha256};
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

#[tokio::test]
async fn complete_delete_deltas_preserve_projection_across_history_sizes() -> Result<()> {
    use crate::backup::{
        provider::{AssetChange, MediaLocation, MediaRef, ReferenceChange},
        telegram_provider::TelegramProvider as Provider,
        telegram_replay::Event,
    };
    for count in [20, 200, 2000] {
        let root = tempfile::tempdir()?;
        let core = open(root.path());
        let target = setup_target(&core);
        let library = &target.scope.library_id;
        let (_stop, rx) = tokio::sync::watch::channel(false);
        let io = Io::new(rx.clone(), rx, None);
        let device = uuid::Uuid::from_u128(9000).to_string();
        let mut creates = vec![];
        let mut history = vec![];
        let mut media: Vec<_> = (1..count).map(|id| doc(id, "video")).collect();
        // Eleven live assets share the original descriptor. Ten will be deleted, one must survive.
        for document in media.iter_mut().take(11) {
            let mut envelope: Envelope = serde_json::from_str(&document.caption)?;
            envelope.asset_id = uuid::Uuid::from_u128(3000 + document.message_id as u128).to_string();
            envelope.device_id.clone_from(&device);
            envelope.container = "mp4".into();
            document.caption = envelope.caption()?;
            let change = AssetChange {
                asset_id: envelope.asset_id, operation_id: envelope.operation_id, device_id: device.clone(),
                base: None, delete: false, recreate: false, fields: [("original_filename".into(), "video.mp4".into())].into(),
                media: Some(MediaRef { scope: target.scope.clone(), location: MediaLocation::TelegramMessage {
                    account_id: target.account_id.clone(), channel_id: target.channel_id.clone(),
                    message_id: i64::from(document.message_id), document_id: document.document_id,
                }, bytes: document.bytes, sha256: envelope.sha256, container: "mp4".into() }),
                cover: ReferenceChange::Unchanged, transcript: ReferenceChange::Unchanged,
                conflicts: vec![], resolved_conflicts: vec![], field_bases: Default::default(),
            };
            history.push(serde_json::json!({
                "revision": (i64::from(document.message_id) << 32) | 1, "body": Event::encode(&target,&change)?,
            }));
            creates.push(change);
        }
        let body = serde_json::to_vec(&serde_json::json!({
            "schema_version": 2, "library_id": library, "provider_type": "telegram", "mode": "single_writer",
            "account_id": target.account_id, "channel_id": target.channel_id, "archived_events": history,
        }))?;
        let initial = ChannelSnapshot {
            high: i64::from(count), media: media.clone(),
            indexes: vec![index(count, "library", library, library, &device, body)],
        };
        let channel = Channel(Mutex::new([Step::Full(100, initial)].into()));
        let mut mapped = refresh(&core, &target, &io, &channel).await?;
        let mut projection = Provider::project_snapshot(&core, &target, &io, None, &mapped)?;
        let mut expected = projection.assets.clone();
        assert_eq!(expected.len(), 11);
        let mut timings = vec![];
        for (offset, create) in (0..10).zip(creates) {
            let video = offset + 1;
            let old_head = count + offset;
            let new_head = old_head + 1;
            let change = AssetChange {
                base: Some(expected[&create.asset_id].clone()), delete: true, media: None, fields: Default::default(),
                operation_id: uuid::Uuid::from_u128(6000 + offset as u128).to_string(), ..create
            };
            let body = Provider::direct_archive_body(&io, &target, &change.asset_id, &mapped, Event::encode(&target, &change)?)?;
            let head = index(new_head, "library", library, &change.operation_id, &device, body);
            let ids = vec![video, old_head, new_head];
            let pts = 100 + offset * 3;
            channel.0.lock().unwrap().extend([
                Step::Changes(pts, Some((pts + 3, ids.clone()))),
                Step::Read(ids, Ok(ChannelSnapshot { indexes: vec![head.clone()], ..Default::default() })),
            ]);
            let started = std::time::Instant::now();
            mapped = refresh(&core, &target, &io, &channel).await?;
            timings.push(started.elapsed().as_micros());
            assert!(channel.0.lock().unwrap().is_empty());
            assert_eq!(mapped.media, media[video as usize..]);
            assert_eq!(mapped.indexes.len(), 1);
            assert_eq!(mapped.indexes[0].document, head.document);
            assert_eq!(mapped.indexes[0].verified_body()?, head.verified_body()?);
            projection = Provider::project_snapshot(&core, &target, &io, Some(&projection.cursor), &mapped)?;
            let tombstone = expected.get_mut(&change.asset_id).unwrap();
            tombstone.deleted = true;
            tombstone.operation_id = change.operation_id;
            tombstone.revision = ((new_head as u64) << 32) | 1;
            assert_eq!(projection.assets, expected);
        }
        timings.sort_unstable();
        // Interpolate at (n - 1) * p, using the same convention as the online samples.
        let p50 = (timings[4] + timings[5]) as f64 / 2.0;
        let p95 = timings[8] as f64 + 0.55 * (timings[9] - timings[8]) as f64;
        println!("offline refresh initial_messages={count} n=10 p50_us={p50:.2} p95_us={p95:.2}");
        core.shutdown().await;
    }
    Ok(())
}

async fn exercise_archive_upgrade(rollover: bool) -> Result<()> {
    use crate::backup::{
        provider::{AssetChange, Checkpoint, MediaLocation, MediaRef, ReferenceChange},
        telegram_provider::{segments::Catalog, TelegramProvider as Provider}, telegram_replay::{Cursor, Event},
        telegram_transfer::tests::send_archive_losing_reply,
        telegram_transfer_journal::{CoreJournal, Journal, SendState},
    };
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
    let mut descriptor = serde_json::json!({"schema_version":2,"library_id":library,
        "provider_type":"telegram","mode":"single_writer","account_id":target.account_id,"channel_id":target.channel_id});
    let mut sealed_peer = None;
    if rollover {
        // A real oversized pre-segmentation archive. No new fields and no test-only size override.
        let mut history = vec![];
        let mut bytes = 0;
        while bytes < 1024 * 1024 {
            let peer = AssetChange { asset_id:uuid::Uuid::new_v4().to_string(), operation_id:uuid::Uuid::new_v4().to_string(),
                delete:true,media:None,fields:Default::default(),..create.clone() };
            let revision = (2_i64 << 32) + 1 + history.len() as i64;
            let record = serde_json::json!({"revision":revision,"body":Event::encode(&target,&peer)?});
            bytes += serde_json::to_vec(&record)?.len();
            history.push(record);
            sealed_peer = Some((peer.asset_id, revision as u64));
        }
        descriptor["archived_events"] = history.into();
    }
    let legacy = index(5,"library",library,library,&device,serde_json::to_vec(&descriptor)?);
    let initial = snapshot(vec![legacy.clone(),
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
    let wire: serde_json::Value = serde_json::from_slice(&body)?;
    if rollover {
        assert_eq!(wire["previous_segment"]["message_id"],5);
        assert_eq!(wire["archived_events"].as_array().unwrap().len(),1,"only this asset moves into the new segment");
        assert!(body.len() < legacy.document.bytes as usize / 50);
    } else { assert!(wire.get("previous_segment").is_none()); }
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
    let catalog = Catalog::read(&target,&both,&io)?;
    assert!(!catalog.superseded(30));
    assert_eq!(catalog.superseded(5),!rollover);
    let mut removed_ids:Vec<_> = both.indexes.iter().map(|i| i.document.message_id).filter(|id| catalog.superseded(*id)).collect();
    removed_ids.push(10);
    let removals = Channel(Mutex::new([Step::Changes(51,Some((52,removed_ids.clone()))),
        Step::Read(removed_ids,Ok(snapshot(vec![])))].into()));
    let only = refresh(&core,&target,&io,&removals).await?;
    assert_eq!(only.indexes.len(),if rollover {2} else {1});
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
    if rollover {
        let missing = snapshot(vec![direct.clone()]);
        assert_eq!(Provider::project_snapshot(&fresh,&fresh_target,&io,None,&missing).err().unwrap().to_string(),"TELEGRAM_INDEX_CHANGED");
        let mut altered = descriptor.clone();
        altered["archived_events"].as_array_mut().unwrap().pop();
        let replaced = index(5,"library",library,library,&device,serde_json::to_vec(&altered)?);
        assert_eq!(Provider::project_snapshot(&fresh,&fresh_target,&io,None,&snapshot(vec![replaced,direct.clone()])).err().unwrap().to_string(),"TELEGRAM_INDEX_CHANGED");
    }
    let kept = if rollover {vec![legacy.clone(), direct]} else {vec![direct]};
    let scan = Channel(Mutex::new([Step::Full(52,snapshot(kept))].into()));
    let only = refresh(&fresh,&fresh_target,&io,&scan).await?;
    let restored = Provider::project_snapshot(&fresh,&fresh_target,&io,None,&only)?;
    assert!(restored.assets[&asset].deleted);
    assert_eq!(restored.assets[&asset].revision,revision);
    assert_eq!(restored.cursor.evidence,applied.cursor.evidence);

    // A later archive freezes the first deletion at the original carrier revision.
    let second = AssetChange {asset_id:uuid::Uuid::new_v4().to_string(),operation_id:uuid::Uuid::new_v4().to_string(),base:None,..change};
    let next = Provider::direct_archive_body(&io,&target,&second.asset_id,&only,Event::encode(&target,&second)?)?;
    let both = [(asset.as_str(),revision),(second.asset_id.as_str(),(39_u64<<32)|1)];
    let updated: serde_json::Value = serde_json::from_slice(&next)?;
    assert_eq!(updated.get("previous_segment"),wire.get("previous_segment"),"updating the active segment keeps its immutable predecessor");
    let head = index(39,"library",library,&second.operation_id,&device,next);
    let next = snapshot(if rollover {vec![legacy,head]} else {vec![head]});
    let catalog = Catalog::read(&target,&next,&io)?;
    catalog.confirm_deletions(&target,&both)?;
    assert!(catalog.confirm_deletions(&target,&[(asset.as_str(),revision+1)]).is_err());
    let rolled = Provider::project_snapshot(&fresh,&fresh_target,&io,Some(&restored.cursor),&next)?;
    assert_eq!(rolled.assets[&asset].revision,revision);
    assert_eq!(rolled.assets[&second.asset_id].revision,(39_u64<<32)|1);
    if let Some((peer, peer_revision)) = sealed_peer {
        assert_eq!(rolled.assets[&peer].revision,peer_revision);
        catalog.confirm_deletions(&fresh_target,&[(peer.as_str(),peer_revision),both[1]])?;
    }
    core.shutdown().await;
    fresh.shutdown().await;
    Ok(())
}
