//! Cross-asset comparisons with immutable, explicitly selected source snapshots.
use super::{Citation, NewNote, Note, NoteOrigin};
use anyhow::{bail, Context, Result};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RevisionRef {
    pub id: String,
    pub revision: u64,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ComparisonPin {
    pub asset_id: String,
    pub document_id: Option<String>,
    pub copywriting: Option<RevisionRef>,
    pub technique: Option<RevisionRef>,
    pub clip_ids: Vec<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ComparisonDocument {
    pub id: String,
    pub version: u64,
    pub opening: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ComparisonAnalysis {
    pub id: String,
    pub revision: u64,
    pub origin: NoteOrigin,
    pub model_name: Option<String>,
    pub human_revised: bool,
    pub excerpt: String,
    pub structure: Option<Vec<String>>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ComparisonClip {
    pub id: String,
    pub file_id: String,
    pub title: String,
    pub start_ms: u64,
    pub end_ms: u64,
    pub document_id: Option<String>,
    pub document_version: Option<u64>,
    pub quote: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ComparisonSource {
    pub pin: ComparisonPin,
    pub title: String,
    pub document: Option<ComparisonDocument>,
    pub copywriting: Option<ComparisonAnalysis>,
    pub technique: Option<ComparisonAnalysis>,
    pub clips: Vec<ComparisonClip>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ComparisonPoint {
    pub text: String,
    pub sources: Vec<ComparisonSource>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NoteComparison {
    pub title: String,
    pub asset_ids: Vec<String>,
    pub points: Vec<ComparisonPoint>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ComparisonPointInput {
    pub text: String,
    pub sources: Vec<ComparisonPin>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NewComparison {
    pub title: String,
    pub asset_ids: Vec<String>,
    pub points: Vec<ComparisonPointInput>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ComparisonEdit {
    pub title: String,
    pub point_texts: Vec<String>,
}

fn excerpt(text: &str, max: usize) -> String {
    text.chars().take(max).collect()
}
fn analysis(
    db: &Connection,
    asset: &str,
    reference: &RevisionRef,
    kind: &str,
) -> Result<ComparisonAnalysis> {
    let note = super::read_note(db, asset, &reference.id)?;
    if note.revision != reference.revision {
        bail!("COMPARISON_SOURCE_CHANGED");
    }
    if note.kind != kind {
        bail!("COMPARISON_INVALID");
    }
    let value = note.analysis.context("COMPARISON_INVALID")?;
    let structure = match value.content {
        super::AnalysisContent::Copywriting {
            opening,
            viewpoint,
            evidence,
            turn,
            ending,
            takeaways,
        } => Some(
            [opening, viewpoint, evidence, turn, ending, takeaways]
                .iter()
                .map(|s| excerpt(s, 500))
                .collect(),
        ),
        super::AnalysisContent::Technique { .. } => None,
    };
    Ok(ComparisonAnalysis {
        id: note.id,
        revision: note.revision,
        origin: note.origin,
        model_name: note.model_name,
        human_revised: value.human_revised,
        excerpt: excerpt(&note.body, 2000),
        structure,
    })
}
pub fn comparison_source(db: &Connection, pin: &ComparisonPin) -> Result<ComparisonSource> {
    let asset = super::asset(db, &pin.asset_id)?;
    if pin.clip_ids.len() > 8
        || pin.clip_ids.iter().collect::<HashSet<_>>().len() != pin.clip_ids.len()
    {
        bail!("COMPARISON_INVALID");
    }
    let document = pin
        .document_id
        .as_ref()
        .map(|id| {
            let doc = super::read_document(db, &pin.asset_id, id)?;
            Ok::<_, anyhow::Error>(ComparisonDocument {
                id: id.clone(),
                version: doc.summary.version,
                opening: excerpt(
                    doc.text
                        .lines()
                        .find(|s| !s.trim().is_empty())
                        .unwrap_or(""),
                    500,
                ),
            })
        })
        .transpose()?;
    let copywriting = pin
        .copywriting
        .as_ref()
        .map(|r| analysis(db, &pin.asset_id, r, "copywriting_analysis"))
        .transpose()?;
    let technique = pin
        .technique
        .as_ref()
        .map(|r| analysis(db, &pin.asset_id, r, "technique_analysis"))
        .transpose()?;
    let available = super::clips(db, &pin.asset_id)?;
    let clips = pin
        .clip_ids
        .iter()
        .map(|id| {
            let c = available
                .iter()
                .find(|c| &c.id == id)
                .context("CLIP_NOT_FOUND")?;
            Ok(ComparisonClip {
                id: c.id.clone(),
                file_id: c.file_id.clone(),
                title: c.title.clone(),
                start_ms: c.start_ms,
                end_ms: c.end_ms,
                document_id: c.document_id.clone(),
                document_version: c.document_version,
                quote: c.quote.as_ref().map(|s| excerpt(s, 500)),
            })
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(ComparisonSource {
        pin: pin.clone(),
        title: asset.title,
        document,
        copywriting,
        technique,
        clips,
    })
}
impl NoteComparison {
    pub fn body(&self) -> String {
        let mut body = format!("# {}\n", self.title);
        for (index, p) in self.points.iter().enumerate() {
            body.push_str(&format!("\n## {}\n{}\n", index + 1, p.text));
            for s in &p.sources {
                body.push_str(&format!("\n来源：{} [{}]\n", s.title, s.pin.asset_id));
                if let Some(d) = &s.document {
                    body.push_str(&format!(
                        "原文首段 [{}] v{}：{}\n",
                        d.id, d.version, d.opening
                    ));
                } else {
                    body.push_str("原文首段：未知\n");
                }
                for (label, n) in [("文案结构", &s.copywriting), ("制作手法", &s.technique)]
                {
                    if let Some(n) = n {
                        body.push_str(&format!(
                            "{} [{}] r{} {}{}：\n{}\n",
                            label,
                            n.id,
                            n.revision,
                            if matches!(n.origin, NoteOrigin::Ai) {
                                "AI"
                            } else {
                                "人工"
                            },
                            n.model_name
                                .as_ref()
                                .map(|m| format!(" {m}"))
                                .unwrap_or_default(),
                            n.excerpt
                        ));
                    } else {
                        body.push_str(&format!("{label}：未知\n"));
                    }
                }
                if s.clips.is_empty() {
                    body.push_str("片段：未知\n");
                }
                for c in &s.clips {
                    body.push_str(&format!(
                        "片段：{} [{}] 文件 [{}] {}–{} ms{}\n",
                        c.title,
                        c.id,
                        c.file_id,
                        c.start_ms,
                        c.end_ms,
                        c.document_id
                            .as_ref()
                            .map(|id| format!(" 文案 [{id}] v{}", c.document_version.unwrap_or(0)))
                            .unwrap_or_default()
                    ));
                }
            }
        }
        body
    }
    fn citations(&self) -> Vec<Citation> {
        let mut citations = Vec::new();
        let mut seen = HashSet::new();
        for p in &self.points {
            for s in &p.sources {
                let mut add = |id: &str, quote: &str| {
                    if !quote.is_empty()
                        && seen.insert((s.pin.asset_id.clone(), id.to_owned(), quote.to_owned()))
                    {
                        citations.push(Citation {
                            asset_id: s.pin.asset_id.clone(),
                            document_id: id.into(),
                            quote: quote.into(),
                        });
                    }
                };
                if let Some(d) = &s.document {
                    add(&d.id, &d.opening);
                }
                for c in &s.clips {
                    if let (Some(id), Some(quote)) = (&c.document_id, &c.quote) {
                        add(id, quote);
                    }
                }
            }
        }
        citations
    }
    fn validate_shape(&self, owner: &str) -> Result<()> {
        if !(2..=4).contains(&self.asset_ids.len())
            || self.asset_ids.iter().collect::<HashSet<_>>().len() != self.asset_ids.len()
            || !self.asset_ids.iter().any(|id| id == owner)
            || self.title.trim().is_empty()
            || self.title.len() > 400
            || self.points.is_empty()
            || self.points.len() > 24
        {
            bail!("COMPARISON_INVALID");
        }
        for p in &self.points {
            super::documents::validate_text(&p.text)?;
            if p.sources.is_empty()
                || p.sources.len() > 4
                || p.sources
                    .iter()
                    .map(|s| &s.pin.asset_id)
                    .collect::<HashSet<_>>()
                    .len()
                    != p.sources.len()
                || p.sources
                    .iter()
                    .any(|s| !self.asset_ids.contains(&s.pin.asset_id))
            {
                bail!("COMPARISON_INVALID");
            }
        }
        super::documents::validate_text(&self.body())?;
        Ok(())
    }
}
pub(super) fn read(db: &Connection, id: &str) -> Result<Option<NoteComparison>> {
    db.query_row(
        "SELECT data FROM note_comparisons WHERE note_id=?1",
        [id],
        |r| r.get::<_, String>(0),
    )
    .optional()?
    .map(|s| serde_json::from_str(&s).context("DATABASE_ERROR"))
    .transpose()
}
pub(super) fn store(db: &Connection, id: &str, value: &NoteComparison) -> Result<()> {
    db.execute("INSERT INTO note_comparisons(note_id,data) VALUES(?1,?2) ON CONFLICT(note_id) DO UPDATE SET data=excluded.data",params![id,serde_json::to_string(value)?])?;
    Ok(())
}
pub fn create_comparison(db: &Connection, owner: &str, input: &NewComparison) -> Result<String> {
    let tx = db.unchecked_transaction()?;
    let points = input
        .points
        .iter()
        .map(|p| {
            Ok(ComparisonPoint {
                text: p.text.clone(),
                sources: p
                    .sources
                    .iter()
                    .map(|pin| comparison_source(&tx, pin))
                    .collect::<Result<_>>()?,
            })
        })
        .collect::<Result<_>>()?;
    let value = NoteComparison {
        title: input.title.trim().into(),
        asset_ids: input.asset_ids.clone(),
        points,
    };
    value.validate_shape(owner)?;
    for id in &value.asset_ids {
        super::require_asset(&tx, id)?;
    }
    let id = super::create_note(
        &tx,
        owner,
        NewNote {
            kind: "comparison",
            body: &value.body(),
            origin: NoteOrigin::Human,
            model_name: None,
            citations: &value.citations(),
        },
    )?;
    store(&tx, &id, &value)?;
    tx.commit()?;
    Ok(id)
}
pub fn edit_comparison(
    db: &Connection,
    owner: &str,
    id: &str,
    input: &ComparisonEdit,
    revision: u64,
) -> Result<()> {
    let tx = db.unchecked_transaction()?;
    let note = super::read_note(&tx, owner, id)?;
    let mut value = note.comparison.context("COMPARISON_REQUIRED")?;
    if input.point_texts.len() != value.points.len() {
        bail!("COMPARISON_INVALID");
    }
    value.title = input.title.trim().into();
    for (p, text) in value.points.iter_mut().zip(&input.point_texts) {
        p.text = text.clone();
    }
    value.validate_shape(owner)?;
    if tx.execute("UPDATE asset_notes SET body=?1,revision=revision+1,updated_at=?2 WHERE id=?3 AND asset_id=?4 AND revision=?5",
        params![value.body(),super::sql_integer(super::now())?,id,owner,super::sql_integer(revision)?])?!=1 { bail!("NOTE_CONFLICT"); }
    store(&tx, id, &value)?;
    tx.commit()?;
    Ok(())
}
pub(super) fn validate_note(db: &Connection, note: &Note) -> Result<()> {
    let Some(value) = &note.comparison else {
        return Ok(());
    };
    value.validate_shape(&note.asset_id)?;
    if note.kind != "comparison"
        || note.analysis.is_some()
        || !matches!(note.origin, NoteOrigin::Human)
        || note.model_name.is_some()
        || note.body != value.body()
        || serde_json::to_value(&note.citations)? != serde_json::to_value(value.citations())?
    {
        bail!("COMPARISON_INVALID");
    }
    for id in &value.asset_ids {
        super::require_asset(db, id)?;
    }
    for p in &value.points {
        for s in &p.sources {
            if s.pin.clip_ids.iter().any(|id| id.is_empty())
                || s.pin.clip_ids.len() > 8
                || s.pin.clip_ids.iter().collect::<HashSet<_>>().len() != s.pin.clip_ids.len()
            {
                bail!("COMPARISON_INVALID");
            }
            // Mutable analysis revisions may advance. Keep the saved old snapshot, never relabel current text as old.
            let mut pin = s.pin.clone();
            for (reference, snapshot, kind) in [
                (&mut pin.copywriting, &s.copywriting, "copywriting_analysis"),
                (&mut pin.technique, &s.technique, "technique_analysis"),
            ] {
                match (reference.as_mut(), snapshot) {
                    (Some(r), Some(saved)) => {
                        let current = super::read_note(db, &pin.asset_id, &r.id)?;
                        if r.id != saved.id
                            || r.revision == 0
                            || r.revision != saved.revision
                            || current.revision < r.revision
                            || current.kind != kind
                            || serde_json::to_value(current.origin)?
                                != serde_json::to_value(saved.origin)?
                            || current.model_name != saved.model_name
                        {
                            bail!("COMPARISON_INVALID");
                        }
                        if saved.excerpt.chars().count() > 2000
                            || match (&saved.structure, kind) {
                                (Some(fields), "copywriting_analysis") => {
                                    fields.len() != 6
                                        || fields.iter().any(|s| s.chars().count() > 500)
                                }
                                (None, "technique_analysis") => false,
                                _ => true,
                            }
                        {
                            bail!("COMPARISON_INVALID");
                        }
                        if current.revision == r.revision
                            && serde_json::to_value(analysis(db, &pin.asset_id, r, kind)?)?
                                != serde_json::to_value(saved)?
                        {
                            bail!("COMPARISON_INVALID");
                        }
                        r.revision = current.revision;
                    }
                    (None, None) => {}
                    _ => bail!("COMPARISON_INVALID"),
                }
            }
            let mut live_pin = pin.clone();
            live_pin.clip_ids.clear();
            let live = comparison_source(db, &live_pin)?;
            if serde_json::to_value(&live.document)? != serde_json::to_value(&s.document)?
                || s.clips.iter().map(|c| &c.id).collect::<Vec<_>>()
                    != s.pin.clip_ids.iter().collect::<Vec<_>>()
            {
                bail!("COMPARISON_INVALID");
            }
            for c in &s.clips {
                let file_owner: String = db.query_row(
                    "SELECT asset_id FROM asset_files WHERE id=?1",
                    [&c.file_id],
                    |r| r.get(0),
                )?;
                if file_owner != pin.asset_id || c.start_ms >= c.end_ms {
                    bail!("COMPARISON_INVALID");
                }
                let clip_owner: Option<String> = db
                    .query_row(
                        "SELECT asset_id FROM asset_clips WHERE id=?1",
                        [&c.id],
                        |r| r.get(0),
                    )
                    .optional()?;
                if let Some(owner) = clip_owner {
                    if owner != pin.asset_id {
                        bail!("COMPARISON_INVALID");
                    }
                    let mut one = live_pin.clone();
                    one.clip_ids = vec![c.id.clone()];
                    if serde_json::to_value(comparison_source(db, &one)?.clips)?
                        != serde_json::to_value(vec![c])?
                    {
                        bail!("COMPARISON_INVALID");
                    }
                }
                if let Some(id) = &c.document_id {
                    let doc = super::read_document(db, &pin.asset_id, id)?;
                    if Some(doc.summary.version) != c.document_version
                        || c.quote.as_ref().is_none_or(|q| !doc.text.contains(q))
                    {
                        bail!("COMPARISON_INVALID");
                    }
                } else if c.quote.is_some() || c.document_version.is_some() {
                    bail!("COMPARISON_INVALID");
                }
            }
        }
    }
    Ok(())
}
