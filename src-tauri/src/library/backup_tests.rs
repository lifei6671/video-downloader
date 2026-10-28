use super::*;
use crate::desktop_core::transcription::TranscriptResult;
use crate::library::{self, Citation, LocalMetadata, NewNote, NoteOrigin};
use serde_json::json;

fn database() -> Connection {
    let mut db = Connection::open_in_memory().unwrap();
    library::initialize(&mut db, Path::new(":memory:")).unwrap();
    db
}
struct Seed {
    asset: String,
    file: String,
    original: String,
    edited: String,
    note: String,
    path: PathBuf,
}
fn seed(db: &Connection, root: &Path) -> Seed {
    let path = root.join("原视频 中文😀.mp4");
    fs::write(&path, b"synthetic media data").unwrap();
    let source = Source::read(&path).unwrap();
    let imported = library::register_local(
        db,
        &source,
        &LocalMetadata {
            width: 160,
            height: 90,
            duration_ms: Some(4000),
        },
    )
    .unwrap();
    let asset = imported.asset_id;
    let file = imported.file_id;
    let first = library::save_recognized(
        db,
        &asset,
        Some(&file),
        &TranscriptResult { timing: None,
            text: "中文原文😀\n正确引用".into(),
            source,
            model_revision: "model-rev".into(),
            model_name: Some("old-model".into()),
            pipeline_revision: "pipeline-rev".into(),
            backend: Some("cpu".into()),
            completed_at: 10,
        },
    )
    .unwrap();
    let edited = library::save_edited(db, &asset, "我的整理稿第一版", 0, Some(&first)).unwrap();
    library::save_edited(db, &asset, "我的整理稿第二版", 1, Some(&first)).unwrap();
    let note = library::create_note(
        db,
        &asset,
        NewNote {
            kind: "hook",
            body: "这是一条学习笔记",
            origin: NoteOrigin::Ai,
            model_name: Some("synthetic-ai"),
            citations: &[Citation {
                asset_id: asset.clone(),
                document_id: first.clone(),
                quote: "正确引用".into(),
            }],
        },
    )
    .unwrap();
    let parent = library::create_category(db, None, "文案学习").unwrap();
    let child = library::create_category(db, Some(&parent), "开场").unwrap();
    library::set_category_membership(db, std::slice::from_ref(&asset), &child, true).unwrap();
    let tag = library::create_tag(db, "节奏").unwrap();
    library::set_tag_membership(db, std::slice::from_ref(&asset), &tag, true).unwrap();
    db.execute(
        "INSERT INTO asset_resources VALUES(?1,'cover',?2)",
        params![asset, b"synthetic cover".to_vec()],
    )
    .unwrap();
    library::record_visit(db, &asset).unwrap();
    Seed {
        asset,
        file,
        original: first,
        edited,
        note,
        path,
    }
}
fn backup(db: &Connection, target: &Path, media: bool) {
    write_backup(capture_backup(db, media).unwrap(), target, &mut |_| Ok(())).unwrap();
}
fn mutate(package: &Path, change: impl FnOnce(&mut serde_json::Value)) {
    let path = package.join("manifest.json");
    let mut value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    change(&mut value);
    let bytes = serde_json::to_vec_pretty(&value).unwrap();
    fs::write(path, &bytes).unwrap();
    fs::write(
        package.join("manifest.sha256"),
        format!("{:x}", Sha256::digest(&bytes)),
    )
    .unwrap();
}

#[test]
fn portable_backup_restores_versions_notes_covers_visits_and_media_without_private_data() {
    for media in [false, true] {
        let root = tempfile::tempdir().unwrap();
        let db = database();
        let original = seed(&db, root.path());
        db.execute_batch("CREATE TABLE private_settings(secret TEXT); INSERT INTO private_settings VALUES('SYNTHETIC_COOKIE_DO_NOT_EXPORT');").unwrap();
        let package = root.path().join("backup");
        backup(&db, &package, media);
        let json = fs::read_to_string(package.join("manifest.json")).unwrap();
        assert!(
            !json.contains("SYNTHETIC_COOKIE")
                && !json.contains("private_settings")
                && !json.contains(&root.path().to_string_lossy().replace('\\', "\\\\")),
            "no settings or original absolute paths"
        );
        // Originals are unavailable on the destination device.
        fs::remove_file(&original.path).unwrap();
        let target = root.path().join("restored");
        let mut restored = database();
        let report = commit_restore(
            &mut restored,
            prepare_restore(&package, &target, &mut |_| Ok(())).unwrap(),
        )
        .unwrap();
        assert_eq!(
            (
                report.assets,
                report.documents,
                report.notes,
                report.missing_media
            ),
            (1, 3, 1, usize::from(!media))
        );
        let doc = library::read_document(&restored, &original.asset, &original.original).unwrap();
        assert_eq!(doc.text, "中文原文😀\n正确引用");
        assert_eq!(doc.model_revision.as_deref(), Some("model-rev"));
        assert_eq!(doc.source_changed, Some(!media));
        assert_eq!(
            library::read_document(&restored, &original.asset, &original.edited)
                .unwrap()
                .summary
                .version,
            1
        );
        let note = library::read_note(&restored, &original.asset, &original.note).unwrap();
        assert_eq!(note.citations[0].document_id, original.original);
        assert_eq!(note.revision, 1);
        assert_eq!(library::categories(&restored).unwrap().len(), 2);
        assert_eq!(library::tags(&restored).unwrap().len(), 1);
        assert_eq!(
            library::asset_detail(&restored, &original.asset)
                .unwrap()
                .category_ids
                .len(),
            1
        );
        assert_eq!(
            library::asset_cover(&restored, &original.asset).unwrap(),
            b"synthetic cover"
        );
        assert_eq!(
            library::workbench_library(&restored).unwrap().recent.total,
            1
        );
        let file = library::asset_file(&restored, &original.asset, &original.file).unwrap();
        assert!(file.path.starts_with(target.canonicalize().unwrap()));
        assert_eq!(file.missing, !media);
        assert!(!file.changed);
        if media {
            assert_eq!(fs::read(file.path).unwrap(), b"synthetic media data");
        }
        assert_eq!(
            library::save_edited(&restored, &original.asset, "继续学习的第三版", 2, None)
                .unwrap()
                .len(),
            36
        );
        assert_eq!(
            restored
                .query_row("SELECT count(*) FROM asset_transcriptions", [], |r| r
                    .get::<_, i64>(0))
                .unwrap(),
            0
        );
    }
}

#[test]
fn bad_versions_graphs_hashes_and_paths_never_publish_restored_data() {
    let root = tempfile::tempdir().unwrap();
    let db = database();
    let s = seed(&db, root.path());
    for case in [
        "version",
        "traversal",
        "absolute",
        "backslash",
        "ads",
        "duplicate",
        "category_cycle",
        "owner",
        "citation",
        "missing_ref",
        "checksum",
        "media_checksum",
        "cover_missing",
    ] {
        let package = root.path().join(format!("package-{case}"));
        backup(&db, &package, true);
        match case {
            "checksum" => {
                fs::write(package.join("manifest.json"), b"{}").unwrap();
            }
            "media_checksum" => {
                let m: Manifest =
                    serde_json::from_slice(&fs::read(package.join("manifest.json")).unwrap())
                        .unwrap();
                fs::write(
                    package.join(&m.files[0].media.as_ref().unwrap().path),
                    b"corrupt",
                )
                .unwrap();
            }
            "cover_missing" => {
                let m: Manifest =
                    serde_json::from_slice(&fs::read(package.join("manifest.json")).unwrap())
                        .unwrap();
                fs::remove_file(package.join(&m.resources[0].payload.as_ref().unwrap().path))
                    .unwrap();
            }
            _ => mutate(&package, |m| match case {
                "version" => m["version"] = json!(999),
                "traversal" => m["files"][0]["media"]["path"] = json!("../outside.mp4"),
                "absolute" => m["files"][0]["media"]["path"] = json!("C:/outside.mp4"),
                "backslash" => m["files"][0]["media"]["path"] = json!("files\\..\\outside.mp4"),
                "ads" => m["files"][0]["media"]["path"] = json!("files/00000001.bin:secret"),
                "duplicate" => {
                    m["files"][0]["media"]["path"] = m["resources"][0]["payload"]["path"].clone()
                }
                "category_cycle" => {
                    let a = m["categories"][0]["id"].clone();
                    let b = m["categories"][1]["id"].clone();
                    m["categories"][0]["parent_id"] = b;
                    m["categories"][1]["parent_id"] = a;
                }
                "owner" => m["documents"][0]["asset_id"] = json!("unknown-owner"),
                "citation" => m["notes"][0]["citations"][0]["quote"] = json!("伪造引用"),
                "missing_ref" => m["documents"][0]["source_document_id"] = json!("does-not-exist"),
                _ => unreachable!(),
            }),
        }
        let target = root.path().join(format!("restore-{case}"));
        let mut dest = database();
        let result = prepare_restore(&package, &target, &mut |_| Ok(()))
            .and_then(|ready| commit_restore(&mut dest, ready));
        assert!(result.is_err(), "{case}");
        assert!(!target.exists(), "{case} published files");
        require_empty_library(&dest).unwrap();
    }
    assert!(s.path.exists());
}

#[test]
fn backup_and_restore_cancellation_source_changes_and_commit_errors_leave_no_partial_output() {
    let root = tempfile::tempdir().unwrap();
    let db = database();
    let seed = seed(&db, root.path());
    fs::write(&seed.path, vec![7u8; 200_000]).unwrap();
    let canceled = root.path().join("canceled");
    let err = write_backup(capture_backup(&db, true).unwrap(), &canceled, &mut |p| {
        if p.bytes > 64 * 1024 {
            bail!("CANCELED");
        }
        Ok(())
    })
    .unwrap_err();
    assert_eq!(err.to_string(), "CANCELED");
    assert!(!canceled.exists());
    let changed = root.path().join("changed");
    let mut modified = false;
    assert!(
        write_backup(capture_backup(&db, true).unwrap(), &changed, &mut |p| {
            if p.bytes > 64 * 1024 && !modified {
                modified = true;
                fs::write(&seed.path, b"changed during copy")?;
            }
            Ok(())
        })
        .is_err()
    );
    assert!(!changed.exists());
    let package = root.path().join("good");
    backup(&db, &package, true);
    let target = root.path().join("restore-canceled");
    assert!(prepare_restore(&package, &target, &mut |p| {
        if p.bytes > 0 {
            bail!("CANCELED");
        }
        Ok(())
    })
    .is_err());
    assert!(!target.exists());
    let target = root.path().join("restore-sql-error");
    let mut dest = database();
    dest.execute_batch("CREATE TRIGGER fail_commit AFTER INSERT ON library_assets BEGIN INSERT INTO asset_tags VALUES(NEW.id,'missing-tag'); END;").unwrap();
    assert!(commit_restore(
        &mut dest,
        prepare_restore(&package, &target, &mut |_| Ok(())).unwrap()
    )
    .is_err());
    require_empty_library(&dest).unwrap();
    assert!(!target.exists());
    let leftovers: Vec<_> = fs::read_dir(root.path())
        .unwrap()
        .filter_map(|p| p.ok())
        .filter(|p| {
            p.file_name()
                .to_string_lossy()
                .starts_with("yingqu-library-")
        })
        .collect();
    assert!(leftovers.is_empty());
}

#[test]
fn nonempty_or_existing_targets_are_never_overwritten_and_untrusted_filenames_are_not_paths() {
    let root = tempfile::tempdir().unwrap();
    let db = database();
    seed(&db, root.path());
    let package = root.path().join("package");
    backup(&db, &package, true);
    assert_eq!(
        write_backup(capture_backup(&db, false).unwrap(), &package, &mut |_| Ok(
            ()
        ))
        .unwrap_err()
        .to_string(),
        "LIBRARY_TARGET_EXISTS"
    );
    let target = root.path().join("already-present");
    fs::create_dir(&target).unwrap();
    fs::write(target.join("user.txt"), b"keep").unwrap();
    assert!(prepare_restore(&package, &target, &mut |_| Ok(())).is_err());
    assert_eq!(fs::read(target.join("user.txt")).unwrap(), b"keep");
    let target = root.path().join("nonempty");
    let mut dest = database();
    library::create_tag(&dest, "尚无视频但已有标签").unwrap();
    assert_eq!(
        commit_restore(
            &mut dest,
            prepare_restore(&package, &target, &mut |_| Ok(())).unwrap()
        )
        .unwrap_err()
        .to_string(),
        "LIBRARY_NOT_EMPTY"
    );
    assert!(!target.exists());
    mutate(&package, |m| {
        m["files"][0]["filename"] = json!("../../escape.mp4");
    });
    let target = root.path().join("safe");
    let mut dest = database();
    commit_restore(
        &mut dest,
        prepare_restore(&package, &target, &mut |_| Ok(())).unwrap(),
    )
    .unwrap();
    let files = library::list_assets(&dest, false, 0, 20).unwrap();
    assert!(library::asset_files(&dest, &files[0].id).unwrap()[0]
        .path
        .starts_with(target.canonicalize().unwrap()));
    assert!(!root.path().join("escape.mp4").exists());
}

#[test]
fn missing_changed_archived_and_relinked_sources_keep_truthful_state() {
    for case in ["missing", "changed", "relinked"] {
        let root = tempfile::tempdir().unwrap();
        let db = database();
        let s = seed(&db, root.path());
        match case {
            "missing" => fs::remove_file(&s.path).unwrap(),
            "changed" => fs::write(&s.path, b"original media changed externally").unwrap(),
            "relinked" => {
                let replacement = root.path().join("replacement.mp4");
                fs::write(&replacement, b"different recording").unwrap();
                library::relink_local(
                    &db,
                    &s.asset,
                    &s.file,
                    &Source::read(&replacement).unwrap(),
                    &LocalMetadata {
                        width: 90,
                        height: 160,
                        duration_ms: Some(2000),
                    },
                )
                .unwrap();
            }
            _ => unreachable!(),
        }
        library::archive_asset(&db, &s.asset, true).unwrap();
        let package = root.path().join("package");
        backup(&db, &package, true);
        let target = root.path().join("restored");
        let mut dest = database();
        let result = commit_restore(
            &mut dest,
            prepare_restore(&package, &target, &mut |_| Ok(())).unwrap(),
        )
        .unwrap();
        assert_eq!(result.missing_media, usize::from(case == "missing"));
        assert!(library::asset(&dest, &s.asset)
            .unwrap()
            .archived_at
            .is_some());
        assert_eq!(
            library::read_document(&dest, &s.asset, &s.original)
                .unwrap()
                .source_changed,
            Some(true)
        );
        let file = library::asset_file(&dest, &s.asset, &s.file).unwrap();
        assert_eq!(file.changed, case == "changed");
        assert_eq!(file.missing, case == "missing");
        assert!(library::workbench_library(&dest)
            .unwrap()
            .recent
            .items
            .is_empty());
    }
    let root = tempfile::tempdir().unwrap();
    let db = database();
    let package = root.path().join("empty-package");
    backup(&db, &package, false);
    let mut dest = database();
    let target = root.path().join("empty");
    assert_eq!(
        commit_restore(
            &mut dest,
            prepare_restore(&package, &target, &mut |_| Ok(())).unwrap()
        )
        .unwrap()
        .assets,
        0
    );
    require_empty_library(&dest).unwrap();
    assert!(transfer::media_name(0, "malicious.exe").ends_with(".mp4"));
    assert!(transfer::media_name(0, "recording.WEBM").ends_with(".WEBM"));
}

#[test]
fn package_payload_directory_cannot_redirect_reads_outside_package() {
    let root = tempfile::tempdir().unwrap();
    let db = database();
    seed(&db, root.path());
    let package = root.path().join("package");
    backup(&db, &package, true);
    let outside = root.path().join("outside");
    fs::rename(package.join("files"), &outside).unwrap();
    let link = package.join("files");
    #[cfg(windows)]
    {
        let output = std::process::Command::new("cmd")
            .args(["/C", "mklink", "/J"])
            .arg(&link)
            .arg(&outside)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "cannot create isolated junction fixture"
        );
    }
    #[cfg(unix)]
    std::os::unix::fs::symlink(&outside, &link).unwrap();
    let result = prepare_restore(&package, &root.path().join("restore"), &mut |_| Ok(()))
        .err()
        .unwrap();
    #[cfg(windows)]
    fs::remove_dir(&link).unwrap();
    #[cfg(unix)]
    fs::remove_file(&link).unwrap();
    assert_eq!(result.to_string(), "LIBRARY_PACKAGE_PATH");
    assert_eq!(fs::read_dir(&outside).unwrap().count(), 2);
}

#[test]
fn stable_ids_must_not_be_empty_even_when_schema_and_foreign_keys_allow_it() {
    let root = tempfile::tempdir().unwrap();
    let db = database();
    seed(&db, root.path());
    for entity in [
        "assets",
        "files",
        "categories",
        "tags",
        "documents",
        "notes",
    ] {
        let package = root.path().join(entity);
        backup(&db, &package, true);
        mutate(&package, |m| {
            if entity == "documents" {
                m["documents"]
                    .as_array_mut()
                    .unwrap()
                    .iter_mut()
                    .find(|d| d["kind"] == "edited" && d["version"] == 2)
                    .unwrap()["id"] = json!("");
                return;
            }
            let mut row = m[entity][0].clone();
            row["id"] = json!("");
            if entity == "files" {
                row["media"] = json!(null);
                row["matches_record"] = json!(false);
            }
            if entity == "categories" {
                row["parent_id"] = json!(null);
                row["name"] = json!("空ID分类");
            }
            if entity == "tags" {
                row["name"] = json!("空ID标签");
            }
            m[entity].as_array_mut().unwrap().push(row);
        });
        let target = root.path().join(format!("restored-{entity}"));
        assert_eq!(
            prepare_restore(&package, &target, &mut |_| Ok(()))
                .err()
                .unwrap()
                .to_string(),
            "LIBRARY_PACKAGE_INVALID",
            "{entity}"
        );
        assert!(!target.exists());
    }
}
