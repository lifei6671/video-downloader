use super::*;
use crate::desktop_core::Core;
use serde_json::json;

#[path = "../../tests/fixtures/mcp_library.rs"]
mod fixture;

fn core(root: &std::path::Path) -> std::sync::Arc<Core> {
    let path = root.join("test.sqlite");
    let mut db = Connection::open(&path).unwrap();
    library::initialize(&mut db, &path).unwrap();
    fixture::seed(&db, root).unwrap();
    crate::settings::load(&db, root.join("downloads")).unwrap();
    fixture::seed_resource_boundary(&db).unwrap();
    drop(db);
    Core::open(
        &path,
        root.join("downloads"),
        "unused-fixture".into(),
        vec![],
        |_| {},
    )
    .unwrap()
}
fn call(core: &Core, method: &str, params: Value) -> Value {
    core.read_mcp_tool(method, params).unwrap()
}

#[test]
fn search_and_taxonomy_pages_match_domain_data_and_actual_counts() {
    let root = tempfile::tempdir().unwrap();
    let core = core(root.path());
    let first = call(&core, "mcp_search_assets", json!({"limit":2}));
    let second = call(
        &core,
        "mcp_search_assets",
        json!({"limit":2,"cursor":first["next_cursor"]}),
    );
    assert_eq!(first["items"].as_array().unwrap().len(), 2);
    assert_eq!(second["items"].as_array().unwrap().len(), 2);
    let ids: std::collections::HashSet<_> = first["items"]
        .as_array()
        .unwrap()
        .iter()
        .chain(second["items"].as_array().unwrap())
        .map(|item| item["asset"]["id"].as_str().unwrap())
        .collect();
    assert_eq!(ids.len(), 4);
    assert!(second["next_cursor"].is_null());
    let desktop = core
        .search_library(library::SearchRequest {
            query: "学习".into(),
            ..Default::default()
        })
        .unwrap();
    let mcp = call(&core, "mcp_search_assets", json!({"query":"学习"}));
    assert_eq!(
        desktop
            .items
            .iter()
            .map(|item| item.asset.id.as_str())
            .collect::<Vec<_>>(),
        mcp["items"]
            .as_array()
            .unwrap()
            .iter()
            .map(|item| item["asset"]["id"].as_str().unwrap())
            .collect::<Vec<_>>()
    );
    assert_eq!(
        call(
            &core,
            "mcp_search_assets",
            json!({"category_ids":["cat-a"]})
        )["items"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    assert_eq!(
        call(&core, "mcp_search_assets", json!({"file_state":"changed"}))["items"][0]["asset"]
            ["id"],
        "asset-c"
    );
    let cats = call(&core, "mcp_list_categories", json!({"limit":1}));
    assert_eq!(cats["categories"][0]["asset_count"], 1);
    assert_eq!(cats["total"], 3);
    let next = call(
        &core,
        "mcp_list_categories",
        json!({"limit":1,"cursor":cats["next_cursor"]}),
    );
    assert_eq!(next["categories"][0]["parent_id"], "cat-a");
    assert_eq!(
        call(&core, "mcp_list_tags", json!({}))["tags"][0]["asset_count"],
        2
    );
    assert_eq!(
        call(&core, "mcp_list_tags", json!({"query":"不存在"}))["total"],
        0
    );
    assert!(
        call(&core, "mcp_search_assets", json!({"query":"不存在"}))["items"]
            .as_array()
            .unwrap()
            .is_empty()
    );
}

#[test]
fn immutable_versions_and_unicode_scalar_slices_have_exact_references() {
    let root = tempfile::tempdir().unwrap();
    let core = core(root.path());
    let current = call(&core, "mcp_read_transcript", json!({"asset_id":"asset-a"}));
    assert_eq!(current["document"]["id"], "doc-a2");
    assert_eq!(current["content"]["length"], 4000);
    assert_eq!(current["content"]["next_offset"], 4000);
    let old = call(
        &core,
        "mcp_read_transcript",
        json!({"asset_id":"asset-a","document_id":"doc-a1","version":1,"offset":1,"length":3}),
    );
    assert_eq!(old["content"]["text"], "😀e\u{301}");
    assert_eq!(
        old["reference"],
        json!({"asset_id":"asset-a","document_id":"doc-a1","document_version":1,"kind":"edited","offset":1,"length":3})
    );
    let total = fixture::TEXT.chars().count();
    let end = call(
        &core,
        "mcp_read_transcript",
        json!({"asset_id":"asset-a","document_id":"doc-a1","offset":total,"length":1}),
    );
    assert_eq!(end["content"]["text"], "");
    assert_eq!(end["content"]["eof"], true);
    assert!(end["content"]["next_offset"].is_null());
    let version3 = core
        .save_library_document("asset-a", "新版本", 2, None)
        .unwrap();
    assert_eq!(
        call(&core, "mcp_read_transcript", json!({"asset_id":"asset-a"}))["document"]["id"],
        version3
    );
    assert_eq!(
        call(
            &core,
            "mcp_read_transcript",
            json!({"asset_id":"asset-a","document_id":"doc-a1"})
        )["content"]["text"],
        fixture::TEXT
    );
    let nul = core
        .save_library_document("asset-b", "甲\0😀", 1, None)
        .unwrap();
    let text = call(
        &core,
        "mcp_read_transcript",
        json!({"asset_id":"asset-b","document_id":nul,"offset":1,"length":2}),
    );
    assert_eq!(text["content"]["text"], "\0😀");
    assert_eq!(text["content"]["total_characters"], 3);
    let full = call(
        &core,
        "mcp_read_transcript",
        json!({"asset_id":"asset-a","document_id":"doc-a2","length":16000}),
    );
    assert_eq!(
        full["content"]["text"].as_str().unwrap().chars().count(),
        16000
    );
}

#[test]
fn notes_are_revision_pinned_and_citations_resolve_to_immutable_versions() {
    let root = tempfile::tempdir().unwrap();
    let core = core(root.path());
    let notes = call(
        &core,
        "mcp_list_notes",
        json!({"asset_id":"asset-a","limit":1}),
    );
    assert_eq!(notes["total"], 2);
    assert_eq!(notes["notes"][0]["origin"], "human");
    let second = call(
        &core,
        "mcp_list_notes",
        json!({"asset_id":"asset-a","limit":1,"cursor":notes["next_cursor"]}),
    );
    assert_eq!(second["notes"][0]["origin"], "ai");
    let note = call(
        &core,
        "mcp_read_note",
        json!({"note_id":"note-a","revision":1,"offset":1,"length":3,"citation_limit":1}),
    );
    assert_eq!(note["content"]["text"], "😀e\u{301}");
    assert_eq!(note["citations"][0]["reference"]["document_version"], 1);
    assert_eq!(note["citations"][0]["reference"]["offset"], 1);
    assert_eq!(note["citations"][0]["reference"]["length"], 3);
    let second = call(
        &core,
        "mcp_read_note",
        json!({"note_id":"note-a","revision":1,"citation_offset":note["next_citation_offset"],"citation_limit":1}),
    );
    assert_eq!(second["citations"][0]["reference"]["asset_id"], "asset-b");
    assert!(second["next_citation_offset"].is_null());
    core.edit_library_note("asset-a", "note-a", "已修订笔记", 1)
        .unwrap();
    assert_eq!(
        core.read_mcp_tool("mcp_read_note", json!({"note_id":"note-a","revision":1})),
        Err(ReadError::RevisionConflict)
    );
    assert_eq!(
        call(&core, "mcp_read_note", json!({"note_id":"note-a"}))["reference"]["revision"],
        2
    );
    let hit = call(&core, "mcp_search_assets", json!({"query":"已修订"}));
    assert_eq!(hit["items"][0]["hit"]["note_id"], "note-a");
    assert_eq!(hit["items"][0]["hit"]["note_revision"], 2);
}

#[test]
fn invalid_ids_ownership_limits_and_cursors_fail_without_losing_reads() {
    let root = tempfile::tempdir().unwrap();
    let core = core(root.path());
    for (method, input, error) in [
        (
            "mcp_get_asset",
            json!({"asset_id":"no"}),
            ReadError::AssetNotFound,
        ),
        (
            "mcp_read_transcript",
            json!({"asset_id":"asset-a","document_id":"doc-b1"}),
            ReadError::DocumentNotFound,
        ),
        (
            "mcp_read_transcript",
            json!({"asset_id":"empty"}),
            ReadError::DocumentNotFound,
        ),
        (
            "mcp_read_transcript",
            json!({"asset_id":"asset-a","document_id":"doc-a1","version":2}),
            ReadError::VersionConflict,
        ),
        (
            "mcp_read_transcript",
            json!({"asset_id":"asset-a","length":16001}),
            ReadError::InvalidInput,
        ),
        (
            "mcp_read_transcript",
            json!({"asset_id":"asset-a","offset":u64::MAX}),
            ReadError::OutOfRange,
        ),
        (
            "mcp_read_note",
            json!({"note_id":"no"}),
            ReadError::NoteNotFound,
        ),
        (
            "mcp_read_note",
            json!({"note_id":"note-a","asset_id":"asset-b"}),
            ReadError::NoteNotFound,
        ),
        (
            "mcp_read_note",
            json!({"note_id":"note-a","citation_offset":3}),
            ReadError::OutOfRange,
        ),
        (
            "mcp_list_notes",
            json!({"asset_id":"asset-a","cursor":"note-other"}),
            ReadError::InvalidCursor,
        ),
        (
            "mcp_list_categories",
            json!({"cursor":"no"}),
            ReadError::InvalidCursor,
        ),
        ("mcp_list_tags", json!({"limit":0}), ReadError::InvalidInput),
        (
            "mcp_search_assets",
            json!({"limit":101}),
            ReadError::InvalidInput,
        ),
        (
            "mcp_search_assets",
            json!({"cursor":{"id":"no","created_at":1}}),
            ReadError::InvalidCursor,
        ),
        (
            "mcp_search_assets",
            json!({"category_ids":["no"]}),
            ReadError::CategoryNotFound,
        ),
        (
            "mcp_search_assets",
            json!({"tag_ids":["no"]}),
            ReadError::TagNotFound,
        ),
        (
            "mcp_get_asset",
            json!({"asset_id":"asset-a","path":"PRIVATE"}),
            ReadError::InvalidInput,
        ),
    ] {
        assert_eq!(core.read_mcp_tool(method, input), Err(error), "{method}");
    }
    assert!(
        call(&core, "mcp_list_notes", json!({"asset_id":"empty"}))["notes"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        call(&core, "mcp_get_asset", json!({"asset_id":"asset-a"}))["asset"]["id"],
        "asset-a"
    );
}

#[test]
fn public_views_omit_internal_paths_urls_and_keep_paged_metadata() {
    let root = tempfile::tempdir().unwrap();
    let core = core(root.path());
    assert!(core
        .settings()
        .profiles
        .iter()
        .any(|profile| profile.cookie_configured
            && profile
                .proxy
                .as_ref()
                .is_some_and(|proxy| proxy.contains("WB205_SYNTHETIC_PROXY_PASSWORD"))));
    let detail = call(
        &core,
        "mcp_get_asset",
        json!({"asset_id":"asset-a","limit":1}),
    );
    assert_eq!(detail["documents"]["total"], 3);
    assert_eq!(detail["documents"]["items"].as_array().unwrap().len(), 1);
    let next = call(
        &core,
        "mcp_get_asset",
        json!({"asset_id":"asset-a","limit":1,"document_cursor":detail["documents"]["next_cursor"]}),
    );
    assert_ne!(
        detail["documents"]["items"][0]["id"],
        next["documents"]["items"][0]["id"]
    );
    assert_eq!(
        call(&core, "mcp_get_asset", json!({"asset_id":"asset-b"}))["files"][0]["missing"],
        true
    );
    for value in [
        detail,
        call(&core, "mcp_search_assets", json!({})),
        call(
            &core,
            "mcp_read_transcript",
            json!({"asset_id":"asset-a","document_id":"doc-rec"}),
        ),
    ] {
        let serialized = value.to_string();
        assert!(!serialized.contains("PRIVATE_"));
        assert!(!serialized.contains("canonical_url"));
        assert!(!serialized.contains("mtime_ns"));
        assert!(!serialized.contains("source_json"));
        assert!(!serialized.contains("WB205_SYNTHETIC"));
    }
}

#[test]
fn long_quotes_are_paged_as_bounded_excerpts_without_losing_source_ranges() {
    let root = tempfile::tempdir().unwrap();
    let core = core(root.path());
    let quote = fixture::TEXT.repeat(2000);
    let note = core
        .create_library_note(
            "asset-a",
            crate::desktop_core::library_api::NoteInput {
                kind: "note".into(),
                body: "引用长文".into(),
                citations: (0..21)
                    .map(|_| library::Citation {
                        asset_id: "asset-a".into(),
                        document_id: "doc-a2".into(),
                        quote: quote.clone(),
                    })
                    .collect(),
            },
        )
        .unwrap();
    let first = call(&core, "mcp_read_note", json!({"note_id":note}));
    assert_eq!(first["citation_total"], 21);
    assert_eq!(first["citations"].as_array().unwrap().len(), 20);
    assert_eq!(first["next_citation_offset"], 20);
    assert_eq!(
        first["citations"][0]["quote_excerpt"]
            .as_str()
            .unwrap()
            .chars()
            .count(),
        120
    );
    assert_eq!(
        first["citations"][0]["reference"]["length"],
        quote.chars().count()
    );
    assert_eq!(first["citations"][0]["quote_truncated"], true);
    assert!(first.to_string().len() < 64 * 1024);
    let last = call(
        &core,
        "mcp_read_note",
        json!({"note_id":note,"revision":1,"citation_offset":20}),
    );
    assert_eq!(last["citations"].as_array().unwrap().len(), 1);
    assert!(last["next_citation_offset"].is_null());
}
