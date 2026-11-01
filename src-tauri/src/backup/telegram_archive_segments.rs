use super::super::telegram_transfer::RemoteDocument;
use super::*;
use std::collections::{BTreeMap, BTreeSet};

pub(super) const SEGMENT_TARGET: usize = 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct SegmentLink {
    message_id: i32,
    sha256: String,
}

pub(super) struct Segment<'a> {
    pub doc: &'a RemoteDocument,
    pub body: &'a [u8],
    pub records: Vec<(i64, Vec<u8>)>,
    pub parent: Option<SegmentLink>,
    sha256: String,
}

pub(in crate::backup) struct Catalog<'a> {
    pub(super) segments: BTreeMap<i32, Segment<'a>>,
    pub(super) sealed: BTreeSet<i32>,
}

impl<'a> Catalog<'a> {
    pub fn read(target: &Target, snapshot: &'a ChannelSnapshot, io: &Io) -> Result<Self> {
        let mut segments = BTreeMap::new();
        for index in &snapshot.indexes {
            io.check()?;
            let doc = &index.document;
            let envelope: Envelope =
                serde_json::from_str(&doc.caption).context("TELEGRAM_INDEX_INVALID")?;
            if envelope.library_id != target.scope.library_id {
                bail!("BACKUP_LIBRARY_MISMATCH");
            }
            if envelope.kind != "library" {
                continue;
            }
            let body = index.verified_body()?;
            let library: Library =
                serde_json::from_slice(body).context("TELEGRAM_INDEX_INVALID")?;
            library.validate(target, &envelope)?;
            let mut records = vec![];
            library.append_archived_events(target, doc.message_id, &mut 0, &mut records)?;
            let segment = Segment {
                doc,
                body,
                records,
                parent: library.previous_segment,
                sha256: envelope.sha256,
            };
            if segments.insert(doc.message_id, segment).is_some() {
                bail!("TELEGRAM_INDEX_INVALID");
            }
        }
        let (_, mut current) = segments
            .last_key_value()
            .context("BACKUP_LIBRARY_MISMATCH")?;
        let mut sealed = BTreeSet::new();
        while let Some(link) = &current.parent {
            if link.message_id >= current.doc.message_id {
                bail!("TELEGRAM_INDEX_INVALID");
            }
            current = segments
                .get(&link.message_id)
                .context("TELEGRAM_INDEX_CHANGED")?;
            if link.sha256 != current.sha256
                || link.sha256 != format!("{:x}", Sha256::digest(current.body))
            {
                bail!("TELEGRAM_INDEX_CHANGED");
            }
            sealed.insert(link.message_id);
        }
        Ok(Self { segments, sealed })
    }

    pub fn superseded(&self, message_id: i32) -> bool {
        self.segments.contains_key(&message_id)
            && message_id != self.head().doc.message_id
            && !self.sealed.contains(&message_id)
    }

    pub(super) fn head(&self) -> &Segment<'a> {
        self.segments
            .last_key_value()
            .expect("validated archive head")
            .1
    }

    pub fn confirm_deletions(&self, target: &Target, deletions: &[(&str, u64)]) -> Result<()> {
        let mut events = vec![];
        for segment in self.segments.values().filter(|s| {
            self.sealed.contains(&s.doc.message_id)
                || s.doc.message_id == self.head().doc.message_id
        }) {
            events.extend(segment.records.iter().cloned());
        }
        let events = merge_events(events)?;
        let high = events.last().map_or(0, |(revision, _)| *revision);
        let projection = telegram_replay::replay(target, high, events, None)?;
        // Obsolete carriers may coexist after a crash or a handoff. Only remove them
        // if every original revision and byte digest survives in the confirmed chain.
        for (id, segment) in &self.segments {
            if self.superseded(*id) {
                for (revision, bytes) in &segment.records {
                    if projection.cursor.evidence.get(revision)
                        != Some(&format!("{:x}", Sha256::digest(bytes)))
                    {
                        bail!("BACKUP_PUBLICATION_UNCONFIRMED");
                    }
                }
            }
        }
        if !deletions.iter().all(|&(asset, revision)| {
            projection
                .assets
                .get(asset)
                .is_some_and(|s| s.deleted && s.revision == revision)
        }) {
            bail!("BACKUP_PUBLICATION_UNCONFIRMED");
        }
        Ok(())
    }
}

impl ArchivePlan {
    /// The old head stays at its original message ID on rollover. Its direct tombstone
    /// therefore keeps the exact revision already observed by other devices.
    pub(super) fn encode(&mut self, target: &Target, limit: usize) -> Result<Vec<u8>> {
        let combined = serde_json::to_vec(&self.library)?;
        if combined.len() <= limit {
            return Ok(combined);
        }
        let (doc, body) = self.previous.last().context("BACKUP_LIBRARY_MISMATCH")?;
        let head: Library = serde_json::from_slice(body)?;
        let mut records = vec![];
        head.append_archived_events(target, doc.message_id, &mut 0, &mut records)?;
        if records.is_empty() {
            return Ok(combined);
        }
        let sealed: BTreeSet<_> = records.iter().map(|(revision, _)| *revision).collect();
        self.library
            .archived_events
            .retain(|event| !sealed.contains(&event.revision));
        self.library.previous_segment = Some(SegmentLink {
            message_id: doc.message_id,
            sha256: format!("{:x}", Sha256::digest(body)),
        });
        Ok(serde_json::to_vec(&self.library)?)
    }
}
