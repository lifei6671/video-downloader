use super::*;
use std::{fs, path::Path};

fn database() -> Connection {
    let mut db = Connection::open_in_memory().unwrap();
    library::initialize(&mut db, Path::new(":memory:")).unwrap();
    db
}
fn seed(db: &Connection) {
    db.execute_batch("INSERT INTO library_assets(id,title,created_at,updated_at) VALUES('a','参考甲😀',1,1),('b','参考乙',1,1);
        INSERT INTO asset_documents(id,asset_id,kind,text,version,revision,created_at) VALUES('d','a','edited','原文😀，值得学习',1,1,1),('e','b','edited','另一个来源',1,1,1);
        INSERT INTO library_tags VALUES('tag','节奏');").unwrap();
}
fn source() -> Value {
    json!({"model_name":"synthetic-model😀","citations":[{"asset_id":"a","document_id":"d","quote":"原文😀"}]})
}
fn note() -> Value {
    json!({"asset_id":"a","body":"AI 学习笔记😀","source":source()})
}
fn write(db: &Connection, method: &str, input: Value) -> Value {
    dispatch(db, method, input).unwrap().value
}

#[test]
fn six_tools_add_separate_content_and_keep_ai_provenance_and_exact_references() {
    let db = database();
    seed(&db);
    let human = library::create_note(
        &db,
        "a",
        library::NewNote {
            kind: "general",
            body: "人工正文",
            origin: library::NoteOrigin::Human,
            model_name: None,
            citations: &[],
        },
    )
    .unwrap();
    let category = write(
        &db,
        "mcp_create_category",
        json!({"name":"开场😀","parent_id":null}),
    )["id"]
        .as_str()
        .unwrap()
        .to_owned();
    for _ in 0..2 {
        write(
            &db,
            "mcp_add_asset_categories",
            json!({"asset_id":"a","ids":[category,category]}),
        );
        write(
            &db,
            "mcp_add_asset_tags",
            json!({"asset_id":"a","ids":["tag","tag"]}),
        );
    }
    for table in ["asset_categories", "asset_tags"] {
        assert_eq!(
            db.query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            1
        );
    }
    let note = write(&db, "mcp_create_note", note())["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let topic = write(
        &db,
        "mcp_create_topic",
        json!({"input":{"title":"研究😀","description":"来源研究"},"source":source()}),
    )["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let idea=write(&db,"mcp_create_idea",json!({"input":{"title":"新选题😀","viewpoint":"观点","body":"独立草稿","references":[{"kind":"video","asset_id":"a","resource_id":"a"},{"kind":"topic","asset_id":null,"resource_id":topic}]},"source":source()}))["id"].as_str().unwrap().to_owned();
    let notes = library::notes(&db, "a").unwrap();
    let ai = notes.iter().find(|n| n.id == note).unwrap();
    assert!(matches!(ai.origin, library::NoteOrigin::Ai));
    assert_eq!(ai.model_name.as_deref(), Some("synthetic-model😀"));
    assert_eq!(ai.citations[0].document_id, "d");
    assert_eq!(
        notes.iter().find(|n| n.id == human).unwrap().body,
        "人工正文"
    );
    assert_eq!(
        db.query_row("SELECT text FROM asset_documents WHERE id='d'", [], |r| {
            r.get::<_, String>(0)
        })
        .unwrap(),
        "原文😀，值得学习"
    );
    let detail = library::library_idea(&db, &idea).unwrap();
    assert_eq!(detail.references.len(), 2);
    assert_eq!(detail.idea.ai_source.unwrap().citations[0].quote, "原文😀");
    library::edit_idea(
        &db,
        &idea,
        &library::IdeaInput {
            title: "人工修改标题".into(),
            viewpoint: "人工观点".into(),
            body: "人工修改".into(),
            references: vec![],
        },
        1,
    )
    .unwrap();
    assert!(library::library_idea(&db, &idea)
        .unwrap()
        .idea
        .ai_source
        .is_some());
    library::edit_research_topic(
        &db,
        &topic,
        &library::TopicInput {
            title: "人工修改专题".into(),
            description: "人工研究".into(),
        },
        1,
    )
    .unwrap();
    assert!(library::research_topic(&db, &topic)
        .unwrap()
        .topic
        .ai_source
        .is_some());
}

#[test]
fn invalid_citations_unknown_fields_and_batch_failures_do_not_partially_write() {
    let db = database();
    seed(&db);
    for (field, value) in [
        ("asset_id", json!("b")),
        ("quote", json!("不存在的引用")),
        ("document_id", json!("missing")),
    ] {
        let mut value_note = note();
        value_note["source"]["citations"][0][field] = value;
        assert_eq!(
            dispatch(&db, "mcp_create_note", value_note)
                .err()
                .unwrap()
                .to_string(),
            "INVALID_CITATION"
        );
    }
    let mut value_note = note();
    value_note["source"]["model_name"] = json!("");
    assert!(dispatch(&db, "mcp_create_note", value_note).is_err());
    let mut value_note = note();
    value_note["id"] = json!("existing-human-id");
    assert!(dispatch(&db, "mcp_create_note", value_note).is_err());
    assert!(library::notes(&db, "a").unwrap().is_empty());
    assert!(dispatch(
        &db,
        "mcp_add_asset_tags",
        json!({"asset_id":"a","ids":["tag","missing"]})
    )
    .is_err());
    assert_eq!(
        db.query_row("SELECT count(*) FROM asset_tags", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
    assert!(dispatch(&db,"mcp_create_idea",json!({"source":source(),"input":{"title":"失败草稿","viewpoint":"","body":"","references":[{"kind":"video","asset_id":"a","resource_id":"b"}]}})).is_err());
    assert!(library::idea_summaries(&db).unwrap().is_empty());
}

#[test]
fn ai_sources_roundtrip_in_v9_and_invalid_or_downgraded_packages_are_rejected() {
    use sha2::{Digest, Sha256};
    let root = tempfile::tempdir().unwrap();
    let db = database();
    seed(&db);
    let topic = write(
        &db,
        "mcp_create_topic",
        json!({"input":{"title":"研究","description":""},"source":source()}),
    )["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let idea=write(&db,"mcp_create_idea",json!({"input":{"title":"选题","viewpoint":"观点","body":"草稿","references":[]},"source":source()}))["id"].as_str().unwrap().to_owned();
    write(&db, "mcp_create_note", note());
    let package = root.path().join("package");
    library::write_backup(
        library::capture_backup(&db, false).unwrap(),
        &package,
        &mut |_| Ok(()),
    )
    .unwrap();
    let original: Value =
        serde_json::from_slice(&fs::read(package.join("manifest.json")).unwrap()).unwrap();
    assert_eq!(original["version"], 9);
    let mut restored = database();
    library::commit_restore(
        &mut restored,
        library::prepare_restore(&package, &root.path().join("restore"), &mut |_| Ok(())).unwrap(),
    )
    .unwrap();
    assert_eq!(
        serde_json::to_value(library::research_topic(&restored, &topic).unwrap().topic).unwrap(),
        serde_json::to_value(library::research_topic(&db, &topic).unwrap().topic).unwrap()
    );
    assert_eq!(
        serde_json::to_value(library::library_idea(&restored, &idea).unwrap().idea).unwrap(),
        serde_json::to_value(library::library_idea(&db, &idea).unwrap().idea).unwrap()
    );
    for bad in ["version", "owner", "model"] {
        let mut value = original.clone();
        match bad {
            "version" => value["version"] = json!(8),
            "owner" => value["ideas"][0]["ai_source"]["citations"][0]["asset_id"] = json!("b"),
            _ => value["research_topics"][0]["ai_source"]["model_name"] = json!(""),
        }
        let bytes = serde_json::to_vec(&value).unwrap();
        fs::write(package.join("manifest.json"), &bytes).unwrap();
        fs::write(
            package.join("manifest.sha256"),
            format!("{:x}", Sha256::digest(&bytes)),
        )
        .unwrap();
        assert!(
            library::prepare_restore(&package, &root.path().join(bad), &mut |_| Ok(())).is_err(),
            "{bad}"
        );
    }
}

#[test]
fn schema_ten_upgrade_rolls_back_then_retries_with_a_consistent_backup() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("upgrade.sqlite");
    let mut db = Connection::open(&path).unwrap();
    library::initialize(&mut db, &path).unwrap();
    library::create_research_topic(
        &db,
        &library::TopicInput {
            title: "保留研究".into(),
            description: "人工内容".into(),
        },
    )
    .unwrap();
    db.execute_batch("ALTER TABLE research_topics DROP COLUMN ai_source_json;UPDATE library_schema SET version=10;").unwrap();
    assert!(library::initialize(&mut db, &path).is_err());
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM pragma_table_info('research_topics') WHERE name='ai_source_json'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    db.execute_batch("ALTER TABLE library_ideas DROP COLUMN ai_source_json;")
        .unwrap();
    let backup = library::initialize(&mut db, &path).unwrap().unwrap();
    let old = Connection::open(backup).unwrap();
    assert_eq!(
        old.query_row("SELECT version FROM library_schema", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        10
    );
    assert_eq!(
        db.query_row("SELECT version FROM library_schema", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        11
    );
    assert_eq!(library::research_topics(&db).unwrap()[0].title, "保留研究");
}
