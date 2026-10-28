//! Synthetic data shared by read contracts and the isolated stdio host.
use anyhow::Result;
use rusqlite::{params, Connection};
use std::path::Path;

pub const TEXT: &str = "甲😀e\u{301}👩\u{200d}💻中文\n尾";

pub fn seed(db: &Connection, root: &Path) -> Result<()> {
    for (id, title, created, archived) in [
        ("asset-a", "学习表达😀", 10, None),
        ("asset-b", "学习镜头", 20, None),
        ("asset-c", "节奏变化", 30, None),
        ("empty", "空资料", 40, None),
        ("archived", "归档学习", 50, Some(50)),
    ] {
        db.execute("INSERT INTO library_assets(id,title,author,canonical_url,created_at,updated_at,archived_at) VALUES(?1,?2,'示例作者','https://example.invalid/media?PRIVATE_SIGNED_URL=secret',?3,?3,?4)",params![id,title,created,archived])?;
    }
    db.execute("INSERT INTO library_assets(id,title,created_at,updated_at,archived_at) VALUES('oversize',?1,60,60,60)", ["x".repeat(1024 * 1024)])?;
    let media = root.join("PRIVATE_MEDIA_PATH.mp4");
    std::fs::write(&media, b"1234")?;
    let changed = root.join("PRIVATE_CHANGED_PATH.mp4");
    std::fs::write(&changed, b"12345")?;
    for (id, asset, path) in [
        ("file-a", "asset-a", media.clone()),
        ("file-b", "asset-b", root.join("PRIVATE_MISSING_PATH.mp4")),
        ("file-c", "asset-c", changed),
    ] {
        db.execute(
            "INSERT INTO asset_files(id,asset_id,path,size) VALUES(?1,?2,?3,4)",
            params![id, asset, path.to_string_lossy()],
        )?;
    }
    db.execute_batch("INSERT INTO library_categories(id,parent_id,name) VALUES('cat-a',NULL,'知识分类😀'),('cat-b','cat-a','镜头手法'),('cat-c',NULL,'空分类');
        INSERT INTO library_tags VALUES('tag-a','学习'),('tag-b','镜头'),('tag-c','空标签');
        INSERT INTO asset_categories VALUES('asset-a','cat-a'),('asset-b','cat-b'),('archived','cat-a');
        INSERT INTO asset_tags VALUES('asset-a','tag-a'),('asset-b','tag-a'),('archived','tag-a');")?;
    for (id, asset, version, text) in [
        ("doc-a1", "asset-a", 1, TEXT.to_owned()),
        ("doc-a2", "asset-a", 2, TEXT.repeat(2000)),
        ("doc-b1", "asset-b", 1, "镜头学习".into()),
    ] {
        db.execute("INSERT INTO asset_documents(id,asset_id,kind,text,version,revision,created_at) VALUES(?1,?2,'edited',?3,?4,?4,1)",params![id,asset,text,version])?;
    }
    let mtime = media
        .metadata()?
        .modified()?
        .duration_since(std::time::UNIX_EPOCH)?
        .as_nanos()
        .to_string();
    let source =
        serde_json::json!({"path":media.canonicalize()?,"size":4,"mtime_ns":mtime}).to_string();
    db.execute("INSERT INTO asset_documents(id,asset_id,file_id,kind,text,version,revision,source_json,model_revision,pipeline_revision,created_at) VALUES('doc-rec','asset-a','file-a','recognized','识别原文😀',1,1,?1,'fixture','fixture',1)",[source])?;
    let citations = serde_json::json!([
        {"asset_id":"asset-a","document_id":"doc-a1","quote":"😀e\u{301}"},
        {"asset_id":"asset-b","document_id":"doc-b1","quote":"镜头"}
    ])
    .to_string();
    db.execute("INSERT INTO asset_notes(id,asset_id,kind,body,origin,citations_json,revision,created_at,updated_at) VALUES('note-a','asset-a','note',?1,'human',?2,1,1,1)",params![TEXT,citations])?;
    db.execute("INSERT INTO asset_notes(id,asset_id,kind,body,origin,model_name,citations_json,revision,created_at,updated_at) VALUES('note-b','asset-a','analysis','AI学习笔记😀','ai','fixture-model','[]',1,2,2)",[])?;
    db.execute("INSERT INTO asset_notes(id,asset_id,kind,body,origin,citations_json,revision,created_at,updated_at) VALUES('note-other','asset-b','note','其他资产笔记','human','[]',1,3,3)",[])?;
    Ok(())
}

pub fn seed_resource_boundary(db: &Connection) -> Result<()> {
    db.execute("INSERT INTO library_assets(id,title,created_at,updated_at,archived_at) VALUES(?1,'URI转义',70,70,70)",["中文 +%😀"])?;
    // Synthetic markers only. settings::load has already created these tables;
    // no OS credential store or daily user data is involved.
    db.execute("INSERT INTO platform_settings(platform,user_agent,proxy,cookie,credential_ref) VALUES('youtube','WB205_SYNTHETIC_USER_AGENT','http://fixture:WB205_SYNTHETIC_PROXY_PASSWORD@127.0.0.1:9','WB205_SYNTHETIC_COOKIE_DO_NOT_EXPOSE',NULL)",[])?;
    Ok(())
}
