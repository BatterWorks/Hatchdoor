//! Read-only link findings from one published Vault generation.

use std::collections::{BTreeMap, BTreeSet};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::{VaultReadCore, VaultReadError, VaultReadProjection, VaultScope};
use crate::api_types::NoteRef;
use crate::cache::vault_snapshots::{NoteBodies, VaultSnapshotRead};
use crate::vault::{NoteEntry, VaultIndex};
use crate::vault_registry::VaultId;

pub const LINK_FINDINGS_DEFAULT_LIMIT: usize = 50;
pub const LINK_FINDINGS_MAX_LIMIT: usize = 200;

#[derive(Clone, Copy, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum LinkFindingKind {
    BrokenLinks,
    Orphans,
    DeadEnds,
}

#[derive(Debug, Serialize, JsonSchema)]
#[serde(untagged)]
pub enum LinkFinding {
    BrokenLink { target: String, notes: Vec<NoteRef> },
    Note(NoteRef),
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct LinkFindingsResponse {
    pub vault_id: VaultId,
    pub kind: LinkFindingKind,
    pub findings: Vec<LinkFinding>,
    pub total: usize,
    pub next_cursor: Option<String>,
}

impl VaultReadCore<'_> {
    /// Lists findings, never a score or a repair plan. Resolution, source
    /// identities and text all come from the same published generation.
    pub fn find_link_problems(
        &self,
        vault_id: VaultId,
        kind: LinkFindingKind,
        limit: Option<usize>,
        cursor: Option<&str>,
    ) -> Result<VaultReadProjection<LinkFindingsResponse>, VaultReadError> {
        let offset = cursor
            .map(str::parse::<usize>)
            .transpose()
            .map_err(|_| VaultReadError {
                code: "invalid_link_findings_cursor".to_string(),
                message: "cursor must be the next_cursor returned by the previous page".to_string(),
                vault_id: Some(vault_id),
                retryable: false,
            })?
            .unwrap_or(0);
        let limit = limit
            .unwrap_or(LINK_FINDINGS_DEFAULT_LIMIT)
            .clamp(1, LINK_FINDINGS_MAX_LIMIT);
        let bodies = match kind {
            LinkFindingKind::BrokenLinks => NoteBodies::Load,
            _ => NoteBodies::Omit,
        };
        let projection =
            self.collection_with(VaultScope::One(vault_id), bodies, |_, _, snapshot| {
                let findings = match kind {
                    LinkFindingKind::BrokenLinks => broken_links(snapshot),
                    LinkFindingKind::Orphans => note_findings(orphan_notes(snapshot)),
                    LinkFindingKind::DeadEnds => {
                        let sources: BTreeSet<&str> = snapshot
                            .links
                            .iter()
                            .map(|link| link.source_slug.as_str())
                            .collect();
                        note_findings(
                            snapshot
                                .notes
                                .iter()
                                .filter(|note| !sources.contains(note.slug.as_str()))
                                .map(|note| NoteRef {
                                    title: note.title.clone(),
                                    slug: note.slug.clone(),
                                })
                                .collect(),
                        )
                    }
                };
                let total = findings.len();
                let end = offset.saturating_add(limit).min(total);
                LinkFindingsResponse {
                    vault_id,
                    kind,
                    total,
                    findings: findings.into_iter().skip(offset).take(limit).collect(),
                    next_cursor: (end < total).then(|| end.to_string()),
                }
            })?;
        Ok(projection.map(|data| {
            data.into_iter()
                .next()
                .expect("one Vault yields one answer")
        }))
    }
}

/// Shared with the statistics page, including its established title ordering.
pub(super) fn orphan_notes(snapshot: &VaultSnapshotRead) -> Vec<NoteRef> {
    let sources: BTreeSet<&str> = snapshot
        .links
        .iter()
        .map(|link| link.source_slug.as_str())
        .collect();
    let targets: BTreeSet<&str> = snapshot
        .links
        .iter()
        .map(|link| link.target_slug.as_str())
        .collect();
    let mut notes: Vec<NoteRef> = snapshot
        .notes
        .iter()
        .filter(|note| {
            !sources.contains(note.slug.as_str()) && !targets.contains(note.slug.as_str())
        })
        .map(|note| NoteRef {
            title: note.title.clone(),
            slug: note.slug.clone(),
        })
        .collect();
    notes.sort_by(|a, b| a.title.cmp(&b.title));
    notes
}

fn note_findings(mut notes: Vec<NoteRef>) -> Vec<LinkFinding> {
    sort_notes(&mut notes);
    notes.into_iter().map(LinkFinding::Note).collect()
}

fn sort_notes(notes: &mut [NoteRef]) {
    notes.sort_by(|a, b| a.title.cmp(&b.title).then_with(|| a.slug.cmp(&b.slug)));
}

fn broken_links(snapshot: &VaultSnapshotRead) -> Vec<LinkFinding> {
    let catalog = VaultIndex::link_resolution_catalog(
        snapshot
            .notes
            .iter()
            .map(|note| NoteEntry {
                title: note.title.clone(),
                slug: note.slug.clone(),
                relative_path: note.relative_path.clone(),
                path: Default::default(),
                layer: note.layer.clone(),
            })
            .collect(),
    );
    let mut grouped: BTreeMap<String, Vec<NoteRef>> = BTreeMap::new();
    for note in &snapshot.notes {
        let Some(content) = snapshot.note_bodies.get(&note.slug) else {
            continue;
        };
        let folder = note
            .relative_path
            .rsplit_once('/')
            .map_or("", |(folder, _)| folder);
        for target in catalog.unresolved_note_targets(content, folder) {
            grouped.entry(target).or_default().push(NoteRef {
                title: note.title.clone(),
                slug: note.slug.clone(),
            });
        }
    }
    grouped
        .into_iter()
        .map(|(target, mut notes)| {
            sort_notes(&mut notes);
            LinkFinding::BrokenLink { target, notes }
        })
        .collect()
}
