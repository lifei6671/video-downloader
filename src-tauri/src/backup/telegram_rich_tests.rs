//! Explicit capability experiment only; never used by the production backup protocol.
use super::*;
use crate::{
    backup::{
        provider::{ProviderType, Scope},
        repository,
        telegram_session::Phase,
    },
    desktop_core::Core,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

#[derive(Deserialize)]
struct Config {
    channel_name: String,
    data_directory: PathBuf,
    work_directory: PathBuf,
    source: PathBuf,
    source_sha256: String,
    original_asset_id: String,
    video_message_id: i32,
}
#[derive(Serialize, Deserialize)]
struct Run {
    target: Target,
    marker: String,
    random_id: i64,
    stage: String,
    message_id: Option<i32>,
}
fn save(path: &Path, run: &Run) -> Result<()> {
    std::fs::write(path, serde_json::to_vec_pretty(run)?)?;
    Ok(())
}
fn caption(text: &str) -> tl::enums::PageCaption {
    tl::types::PageCaption {
        text: tl::types::TextPlain { text: text.into() }.into(),
        credit: tl::types::TextPlain {
            text: String::new(),
        }
        .into(),
    }
    .into()
}
struct OriginalDocument(tl::types::Document);
impl grammers_client::media::Downloadable for OriginalDocument {
    fn to_raw_input_location(&self) -> Option<tl::enums::InputFileLocation> {
        Some(
            tl::types::InputDocumentFileLocation {
                id: self.0.id,
                access_hash: self.0.access_hash,
                file_reference: self.0.file_reference.clone(),
                thumb_size: String::new(),
            }
            .into(),
        )
    }
    fn size(&self) -> Option<usize> {
        usize::try_from(self.0.size).ok()
    }
    fn dc_id(&self) -> Option<i32> {
        Some(self.0.dc_id)
    }
}

#[tokio::test]
#[ignore = "explicit dedicated session/channel; publishes at most one rich validation message using existing original documents"]
async fn telegram_rich_live_four_original_attachments() -> Result<()> {
    let cfg: Config = serde_json::from_slice(&std::fs::read(
        std::env::var_os("YINGQU_TELEGRAM_RICH_TEST_CONFIG")
            .context("explicit rich validation config required")?,
    )?)?;
    if cfg.data_directory.file_name().and_then(|s| s.to_str())
        != Some("me.disign.yingqu.telegram-validation")
        || cfg.work_directory.file_name().and_then(|s| s.to_str())
            != Some("telegram-rich-validation")
        || cfg.channel_name.trim().is_empty()
        || cfg.video_message_id <= 0
    {
        bail!("dedicated validation session/channel required");
    }
    uuid::Uuid::parse_str(&cfg.original_asset_id)?;
    if repository::fingerprint(&cfg.source)?.0 != cfg.source_sha256 {
        bail!("original video changed");
    }
    std::fs::create_dir_all(&cfg.work_directory)?;
    let core = Core::open(
        &cfg.data_directory.join("yingqu.sqlite"),
        cfg.work_directory.clone(),
        "unused".into(),
        vec![],
        |_| {},
    )?;
    let original_path = cfg.source.clone();
    let original_hash = cfg.source_sha256.clone();
    let result=async {
        let binding=core.telegram_accounts.binding(crate::telegram_accounts::Purpose::Backup);
        let login=binding.session.resume(binding.namespace).await?;
        if !matches!(login.phase,Phase::Ready) {bail!("existing authorized session required");}
        let id=login.operation_id.context("ready operation required")?;
        let account=login.account.context("ready account required")?;
        let channels=core.telegram_accounts.binding(crate::telegram_accounts::Purpose::Backup).session.channels(&id).await?;
        let matching:Vec<_>=channels.iter().filter(|c|c.title==cfg.channel_name && c.can_post).collect();
        if matching.len()!=1 {bail!("one authorized writable private channel required");}
        let path=cfg.work_directory.join("run.json");
        let mut run:Run=if path.exists() {serde_json::from_slice(&std::fs::read(&path)?)?} else {
            Run { target:Target {scope:Scope {space_id:uuid::Uuid::new_v4().to_string(),library_id:uuid::Uuid::new_v4().to_string(),provider_type:ProviderType::Telegram,protocol:2},account_id:account.id.clone(),channel_id:matching[0].id.clone()},marker:format!("映取富消息验证 {}",uuid::Uuid::new_v4()),random_id:i64::from_le_bytes(uuid::Uuid::new_v4().as_bytes()[..8].try_into()?),stage:"prepared".into(),message_id:None }
        };
        if run.target.account_id!=account.id || run.target.channel_id!=matching[0].id {bail!("validation target changed");}
        save(&path,&run)?;
        let (_cancel,rx)=tokio::sync::watch::channel(false);
        let report=core.telegram_accounts.binding(crate::telegram_accounts::Purpose::Backup).session.with_transport(&core,&id,run.target.clone(),rx,None,move |_,transport,io|Box::pin(async move {
            let mut originals=BTreeMap::<String,(tl::types::Document,Envelope)>::new();
            let video=io.run(transport.message(cfg.video_message_id)).await?;
            let mut history=transport.client.iter_messages(transport.peer).limit(500);
            let mut source_messages=vec![video];
            while let Some(message)=io.run(async {history.next().await.map_err(rpc_error)}).await? {
                io.touch();
                let Ok(env)=serde_json::from_str::<Envelope>(message.text()) else {continue;};
                if env.asset_id==cfg.original_asset_id && matches!(env.kind.as_str(),"cover"|"transcript"|"event") {source_messages.push(message);}
            }
            for message in source_messages {
                let env:Envelope=serde_json::from_str(message.text())?;
                if originals.contains_key(&env.kind) {continue;}
                let (_,Media::Document(media))=document(&message)? else {bail!("original document required");};
                let Some(tl::enums::Document::Document(doc))=media.raw.document else {bail!("original document missing");};
                if doc.size!=i64::try_from(env.bytes)? {bail!("original document size changed");}
                originals.insert(env.kind.clone(),(doc,env));
            }
            if originals.len()!=4 || !["video","cover","transcript","event"].iter().all(|k|originals.contains_key(*k)) {bail!("four original attachments required");}
            if originals["video"].1.sha256!=cfg.source_sha256 {bail!("video receipt differs from original");}
            if run.stage=="rejected" {return Ok(serde_json::from_slice(&std::fs::read(cfg.work_directory.join("rich-report.json"))?)?);}
            if run.message_id.is_none() && run.stage!="prepared" {bail!("previous send is uncertain; inspect before any resend");}
            if run.message_id.is_none() {
                let mut blocks=vec![tl::types::PageBlockParagraph {text:tl::types::TextPlain {text:run.marker.clone()}.into()}.into(),tl::types::PageBlockVideo {autoplay:false,r#loop:false,spoiler:false,video_id:originals["video"].0.id,caption:caption("原视频")}.into()];
                for kind in ["cover","transcript","event"] {blocks.push(tl::types::PageBlockDocument {document_id:originals[kind].0.id,caption:caption(kind)}.into());}
                let documents=originals.values().map(|(d,_)|tl::types::InputDocument {id:d.id,access_hash:d.access_hash,file_reference:d.file_reference.clone()}.into()).collect();
                run.stage="sending".into();save(&path,&run)?;
                let request=tl::functions::messages::SendMessage {
                    no_webpage:true,silent:true,background:false,clear_draft:false,noforwards:false,update_stickersets_order:false,invert_media:false,allow_paid_floodskip:false,peer:transport.peer.into(),reply_to:None,message:run.marker.clone(),random_id:run.random_id,reply_markup:None,entities:None,schedule_date:None,schedule_repeat_period:None,send_as:None,quick_reply_shortcut:None,effect:None,allow_paid_stars:None,suggested_post:None,
                    rich_message:Some(tl::types::InputRichMessage {rtl:false,noautolink:true,blocks,photos:None,documents:Some(documents),users:None}.into()),
                };
                let updates=match io.run(async {transport.client.invoke(&request).await.map_err(anyhow::Error::new)}).await {
                    Ok(updates)=>updates,
                    Err(error)=> {
                        if let Some(InvocationError::Rpc(rpc))=error.downcast_ref::<InvocationError>() {
                            run.stage="rejected".into();save(&path,&run)?;
                            let report=serde_json::json!({"outcome":"SERVER_REJECTED","server_error":rpc.name,"single_message_verified":false,"attachments_planned":4,"original_documents_reused":true,"transcoded":false});
                            std::fs::write(cfg.work_directory.join("rich-report.json"),serde_json::to_vec_pretty(&report)?)?;
                            return Ok(report);
                        }
                        return Err(error);
                    }
                };
                run.message_id=sent_id(updates,run.random_id);run.stage="sent".into();save(&path,&run)?;
            }
            let message_id=run.message_id.context("sent message id unavailable; inspect before resend")?;
            let message=io.run(transport.message(message_id)).await?;
            let tl::enums::Message::Message(message)=message.raw else {bail!("rich message missing");};
            let rich=message.rich_message.context("server did not retain rich content")?;
            let tl::enums::RichMessage::Message(rich)=rich;
            if rich.part || rich.documents.len()!=4 || message.grouped_id.is_some() {bail!("full single-message content not confirmed");}
            let mut checks=vec![];
            for doc in rich.documents {
                let tl::enums::Document::Document(doc)=doc else {bail!("rich document missing");};
                let (kind,(_,expected))=originals.iter().find(|(_, (d,_))|d.id==doc.id).context("unexpected rich attachment")?;
                let mut download=transport.client.iter_download(&OriginalDocument(doc));let mut hash=Sha256::new();let mut bytes=0u64;
                while let Some(chunk)=io.run(async {download.next().await.map_err(rpc_error)}).await? {io.touch();bytes+=chunk.len() as u64;hash.update(&chunk);}
                if bytes!=expected.bytes || format!("{:x}",hash.finalize())!=expected.sha256 {bail!("rich attachment original bytes differ");}
                checks.push(serde_json::json!({"kind":kind,"bytes":bytes,"sha256":expected.sha256}));
            }
            run.stage="confirmed".into();save(&path,&run)?;
            let report=serde_json::json!({"outcome":"CONFIRMED","single_message_verified":true,"attachment_count":4,"all_original_hashes_match":true,"original_documents_reused":true,"transcoded":false,"attachments":checks});
            std::fs::write(cfg.work_directory.join("rich-report.json"),serde_json::to_vec_pretty(&report)?)?;
            std::fs::write(cfg.work_directory.join("playback-target.json"),serde_json::to_vec_pretty(&serde_json::json!({"channel_id":run.target.channel_id,"message_id":message_id,"marker":run.marker}))?)?;
            Ok(report)
        })).await?;
        println!("rich validation: {}",report["outcome"]);
        Ok::<(),anyhow::Error>(())
    }.await;
    let unchanged = repository::fingerprint(&original_path).map(|(hash, _)| hash == original_hash);
    core.shutdown().await;
    if !unchanged? {
        bail!("original video changed");
    }
    result
}
