//! Vault-qualified snapshot storage for the shared disposable SQLite cache.
#![allow(dead_code)] // #92 is the first shared-core consumer of this internal cache seam.

use std::collections::BTreeMap;

use rusqlite::{OptionalExtension, Transaction, params};

use crate::cache::{BuildOptions, SqliteCache};
use crate::embed::Embedder;
use crate::vault::{NoteMetadata, VaultIndex};
use crate::vault_registry::VaultId;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum VaultSnapshotFreshness {
    Fresh,
    Stale,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct VaultSnapshotStatus {
    pub(crate) participating: bool,
    pub(crate) freshness: VaultSnapshotFreshness,
    /// Whether this generation carries vectors, and so can answer semantic
    /// search. A Vault's first Index turn publishes its structural rows before
    /// embedding starts, so browsing does not wait on the embedder; that
    /// generation is `participating` and `Fresh` (its notes/links/tags do
    /// describe current Markdown) but not yet `searchable`.
    ///
    /// Deliberately a separate axis from [`VaultSnapshotFreshness`]: a
    /// structure-only generation can itself go stale while a later Index turn
    /// rebuilds it, so folding the two into one enum would produce states no
    /// caller could represent.
    pub(crate) searchable: bool,
}

/// One Vault's complete published read snapshot. This is intentionally a
/// cache-local representation: callers must treat it as disposable data and
/// keep exact note reads on the authoritative Markdown path.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct VaultSnapshotRead {
    pub(crate) notes: Vec<VaultSnapshotNote>,
    pub(crate) links: Vec<VaultSnapshotLink>,
    pub(crate) tags_by_note: BTreeMap<String, Vec<String>>,
    pub(crate) chunks: Vec<VaultSnapshotChunk>,
    /// This Vault's declared layer catalog (name + optional description), as
    /// published alongside this generation's other snapshot rows. Sourced
    /// from `.hatchdoor-layer` markers at populate time, not inferred from
    /// current note counts, so a declared layer with zero notes still
    /// appears here. Empty for a generation published before this key
    /// existed, or a Vault with no layer markers.
    pub(crate) layer_catalog: Vec<crate::search::LayerInfo>,
}

/// A participant state and every row used to project it, read from one pinned
/// SQLite snapshot. `None` means there is no currently participating snapshot.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PublishedVaultSnapshot {
    pub(crate) status: VaultSnapshotStatus,
    pub(crate) read: VaultSnapshotRead,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct VaultSnapshotNote {
    pub(crate) title: String,
    pub(crate) slug: String,
    pub(crate) relative_path: String,
    pub(crate) content: String,
    pub(crate) size_bytes: i64,
    pub(crate) mtime_ns: i64,
    pub(crate) layer: Option<String>,
    pub(crate) metadata: NoteMetadata,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct VaultSnapshotChunk {
    pub(crate) chunk_id: i64,
    pub(crate) note_slug: String,
    pub(crate) heading_path: Option<String>,
    pub(crate) content: String,
    pub(crate) layer: Option<String>,
    /// Demoted chunks can intentionally remain keyword-only when the Index
    /// turn bound `HATCHDOOR_EMBED_LAYERS=false`.
    pub(crate) embedding: Option<Vec<u8>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct VaultSnapshotLink {
    pub(crate) source_slug: String,
    pub(crate) target_slug: String,
}

enum SnapshotRelation {
    Notes,
    Links,
    Chunks,
}

impl SnapshotRelation {
    fn table(self) -> &'static str {
        match self {
            Self::Notes => "vault_notes",
            Self::Links => "vault_note_links",
            Self::Chunks => "vault_chunks",
        }
    }
}

impl SqliteCache {
    /// Build one Vault's candidate index and replace only that Vault's shared
    /// cache rows in one transaction. A failed candidate leaves the prior
    /// searchable snapshot in place and marks it stale.
    pub(crate) fn replace_vault_snapshot(
        &self,
        vault_id: VaultId,
        index: &VaultIndex,
        embedder: &dyn Embedder,
    ) -> Result<(), String> {
        self.replace_vault_snapshot_with_embed_layers(vault_id, index, embedder, true)
    }

    /// Build and publish one Vault's candidate snapshot with its operation's
    /// already-bound embedding-layer policy. The default publisher remains
    /// all-layer embedding for callers outside runtime composition.
    pub(crate) fn replace_vault_snapshot_with_embed_layers(
        &self,
        vault_id: VaultId,
        index: &VaultIndex,
        embedder: &dyn Embedder,
        embed_layers: bool,
    ) -> Result<(), String> {
        let _epoch = self
            .snapshot_model_epoch
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        // Vault snapshots share one sqlite-vec table, so their vectors must
        // have one embedding identity and dimension. A model change wipes the
        // disposable cache before even a single Vault is rebuilt; otherwise a
        // partial rebuild could make global semantic search compare unrelated
        // vector spaces.
        self.reset_if_embedder_changed(embedder)?;
        let attempt = self.begin_vault_snapshot_attempt(vault_id)?;
        let result = (|| {
            let candidate = SqliteCache::in_memory(embedder.embedding_dim())?;
            // Seed the empty candidate with this Vault's published rows so the
            // build's own reuse machinery can find them. Without this the
            // candidate starts empty, every content hash misses, and an
            // unchanged Vault re-embeds from scratch on every Index turn —
            // including on every restart. Purely an optimisation: on failure the
            // seeding transaction rolls back and the build proceeds from empty.
            if let Err(error) = self.seed_candidate_from_snapshot(vault_id, &candidate) {
                tracing::warn!(
                    %vault_id,
                    %error,
                    "could not seed the candidate cache from the published snapshot; rebuilding this Vault from scratch"
                );
            }
            candidate.replace_with_options(
                index,
                embedder,
                None,
                embed_layers,
                &BuildOptions::default(),
            )?;
            self.publish_vault_candidate(vault_id, attempt, &candidate, &embedder.identity(), true)
        })();
        if result.is_err() {
            self.mark_vault_snapshot_stale_if_current(vault_id, attempt)?;
        }
        result
    }

    /// Publish this Vault's structural rows (notes, links, tags, headings,
    /// chunk text) ahead of any embedding work, so browsing a Vault does not
    /// wait on the minutes of vector building that searching it does.
    ///
    /// Returns whether a generation was published. This is a no-op for a Vault
    /// that already has a searchable snapshot: replacing one with a
    /// structure-only generation would take working search away for the length
    /// of a rebuild, which is strictly worse than serving the prior generation
    /// marked stale. So this only ever fires on a Vault's first successful
    /// index (or its first after a model change wiped the cache).
    ///
    /// The chunker needs the embedder's tokenizer, so this still waits for the
    /// model to load. It does not wait for the model to *run*, which is the
    /// part measured in minutes.
    pub(crate) fn publish_vault_structure_snapshot(
        &self,
        vault_id: VaultId,
        index: &VaultIndex,
        embedder: &dyn Embedder,
        embed_layers: bool,
    ) -> Result<bool, String> {
        let _epoch = self
            .snapshot_model_epoch
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        self.reset_if_embedder_changed(embedder)?;
        if self.has_searchable_snapshot(vault_id)? {
            return Ok(false);
        }
        let attempt = self.begin_vault_snapshot_attempt(vault_id)?;
        let result = (|| {
            let candidate = SqliteCache::in_memory(embedder.embedding_dim())?;
            // No seeding: by the check above there is no searchable generation
            // to reuse vectors from, which is all seeding exists for.
            candidate.replace_with_options(
                index,
                embedder,
                None,
                embed_layers,
                &BuildOptions {
                    embed: false,
                    ..BuildOptions::default()
                },
            )?;
            self.publish_vault_candidate(vault_id, attempt, &candidate, &embedder.identity(), false)
        })();
        if result.is_err() {
            self.mark_vault_snapshot_stale_if_current(vault_id, attempt)?;
        }
        result.map(|()| true)
    }

    /// Deliberately does not filter on `participating`. Retirement keeps a
    /// disabled Vault's rows and only clears participation, so a disabled
    /// Vault sits at `(participating = 0, searchable = 1)` with every vector
    /// intact. Treating that as "no searchable generation" would fire the
    /// structure pass on re-enable, whose publication deletes those vectors
    /// and forces a full re-embed of the whole Vault — twice wrong, because
    /// seeding would then find nothing to reuse either.
    fn has_searchable_snapshot(&self, vault_id: VaultId) -> Result<bool, String> {
        let conn = self.read()?;
        Ok(conn
            .query_row(
                "SELECT 1 FROM vault_snapshots WHERE vault_id = ?1 AND searchable = 1",
                params![vault_id.to_string()],
                |row| row.get::<_, i64>(0),
            )
            .optional()
            .map_err(|error| format!("look up searchable Vault snapshot {vault_id}: {error}"))?
            .is_some())
    }

    /// Mark a retained Vault snapshot stale: for the duration of an active
    /// Index turn's scan/candidate-build, so concurrent reads do not report a
    /// retained snapshot as fresh while it is known to be behind the
    /// authoritative Markdown; and when rebuilding fails before
    /// candidate-cache publication begins, so it stays stale afterward. A
    /// no-op (no error) when the Vault has no existing snapshot row.
    pub(crate) fn mark_vault_snapshot_stale(&self, vault_id: VaultId) -> Result<(), String> {
        let attempt = self.begin_vault_snapshot_attempt(vault_id)?;
        self.mark_vault_snapshot_stale_if_current(vault_id, attempt)
    }

    /// Remove a Vault from shared-cache participation without deleting its
    /// last successful rows. Re-enabling is intentionally coupled to a later
    /// successful [`Self::replace_vault_snapshot`] publication.
    pub(crate) fn disable_vault_snapshot(&self, vault_id: VaultId) -> Result<(), String> {
        let conn = self.connection()?;
        conn.execute(
            "UPDATE vault_snapshots SET participating = 0 WHERE vault_id = ?1",
            params![vault_id.to_string()],
        )
        .map_err(|error| format!("disable Vault snapshot {vault_id}: {error}"))?;
        Ok(())
    }

    /// Forget exactly one Vault's disposable shared-cache rows.
    pub(crate) fn disconnect_vault_snapshot(&self, vault_id: VaultId) -> Result<(), String> {
        let mut conn = self.connection()?;
        let tx = conn
            .transaction()
            .map_err(|error| format!("start Vault snapshot disconnect: {error}"))?;
        delete_vault_snapshot(&tx, &vault_id.to_string())?;
        tx.commit()
            .map_err(|error| format!("commit Vault snapshot disconnect: {error}"))
    }

    pub(crate) fn snapshot_note_count(&self, vault_id: VaultId) -> Result<usize, String> {
        self.snapshot_count(SnapshotRelation::Notes, vault_id)
    }

    pub(crate) fn snapshot_link_count(&self, vault_id: VaultId) -> Result<usize, String> {
        self.snapshot_count(SnapshotRelation::Links, vault_id)
    }

    pub(crate) fn snapshot_chunk_count(&self, vault_id: VaultId) -> Result<usize, String> {
        self.snapshot_count(SnapshotRelation::Chunks, vault_id)
    }

    pub(crate) fn snapshot_note_content(
        &self,
        vault_id: VaultId,
        slug: &str,
    ) -> Result<Option<String>, String> {
        let conn = self.read()?;
        conn.query_row(
            "SELECT content FROM vault_notes WHERE vault_id = ?1 AND slug = ?2",
            params![vault_id.to_string(), slug],
            |row| row.get(0),
        )
        .optional()
        .map_err(|error| format!("read Vault snapshot note {vault_id}/{slug}: {error}"))
    }

    pub(crate) fn snapshot_status(
        &self,
        vault_id: VaultId,
    ) -> Result<Option<VaultSnapshotStatus>, String> {
        let conn = self.read()?;
        Self::read_snapshot_status(&conn, vault_id)
    }

    /// Enumerate every Vault with disposable snapshot state so current
    /// registry reconciliation can remove disconnected orphans.
    pub(crate) fn snapshot_vault_ids(&self) -> Result<Vec<VaultId>, String> {
        let conn = self.read()?;
        let mut statement = conn
            .prepare("SELECT vault_id FROM vault_snapshots ORDER BY vault_id")
            .map_err(|error| format!("prepare snapshot Vault IDs: {error}"))?;
        let rows = statement
            .query_map([], |row| row.get::<_, String>(0))
            .map_err(|error| format!("query snapshot Vault IDs: {error}"))?;
        rows.map(|row| {
            row.map_err(|error| format!("read snapshot Vault ID: {error}"))?
                .parse()
                .map_err(|error| format!("parse snapshot Vault ID: {error}"))
        })
        .collect()
    }

    /// Read one participating Vault's state and projection rows from one
    /// SQLite snapshot. This prevents a concurrent publish or disconnect from
    /// pairing a prior freshness value with empty or newer rows.
    pub(crate) fn read_vault_snapshot(
        &self,
        vault_id: VaultId,
    ) -> Result<Option<PublishedVaultSnapshot>, String> {
        let conn = self.read()?;
        conn.execute_batch("BEGIN")
            .map_err(|error| format!("begin Vault snapshot read: {error}"))?;
        let result = (|| {
            let Some(status) = Self::read_snapshot_status(&conn, vault_id)? else {
                return Ok(None);
            };
            if !status.participating {
                return Ok(None);
            }
            let read = Self::read_vault_snapshot_rows(&conn, vault_id)?;
            Ok(Some(PublishedVaultSnapshot { status, read }))
        })();
        let close = if result.is_ok() {
            conn.execute_batch("COMMIT")
                .map_err(|error| format!("commit Vault snapshot read: {error}"))
        } else {
            conn.execute_batch("ROLLBACK")
                .map_err(|error| format!("rollback Vault snapshot read: {error}"))
        };
        close?;
        result
    }

    /// Read one participating snapshot using the caller's already-pinned
    /// SQLite read transaction. Search uses this with its KNN/FTS query so
    /// projection metadata and hits always describe one generation.
    pub(crate) fn read_vault_snapshot_on(
        conn: &rusqlite::Connection,
        vault_id: VaultId,
    ) -> Result<Option<PublishedVaultSnapshot>, String> {
        let Some(status) = Self::read_snapshot_status(conn, vault_id)? else {
            return Ok(None);
        };
        if !status.participating {
            return Ok(None);
        }
        Ok(Some(PublishedVaultSnapshot {
            status,
            read: Self::read_vault_snapshot_rows(conn, vault_id)?,
        }))
    }

    fn read_snapshot_status(
        conn: &rusqlite::Connection,
        vault_id: VaultId,
    ) -> Result<Option<VaultSnapshotStatus>, String> {
        let row: Option<(i64, String, i64)> = conn
            .query_row(
                "SELECT participating, freshness, searchable FROM vault_snapshots \
                 WHERE vault_id = ?1",
                params![vault_id.to_string()],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()
            .map_err(|error| format!("read Vault snapshot status {vault_id}: {error}"))?;
        row.map(|(participating, freshness, searchable)| {
            let freshness = match freshness.as_str() {
                "fresh" => VaultSnapshotFreshness::Fresh,
                "stale" => VaultSnapshotFreshness::Stale,
                unexpected => {
                    return Err(format!(
                        "read Vault snapshot status {vault_id}: unexpected freshness {unexpected:?}"
                    ));
                }
            };
            Ok(VaultSnapshotStatus {
                participating: participating != 0,
                freshness,
                searchable: searchable != 0,
            })
        })
        .transpose()
    }

    fn read_vault_snapshot_rows(
        conn: &rusqlite::Connection,
        vault_id: VaultId,
    ) -> Result<VaultSnapshotRead, String> {
        let vault_id = vault_id.to_string();
        let mut notes_statement = conn
            .prepare(
                "SELECT title, slug, relative_path, content, size_bytes, mtime_ns, layer, \
                 aliases_json, frontmatter_json \
                 FROM vault_notes WHERE vault_id = ?1 ORDER BY relative_path",
            )
            .map_err(|error| format!("prepare Vault snapshot notes: {error}"))?;
        let mut notes = notes_statement
            .query_map(params![&vault_id], |row| {
                Ok(VaultSnapshotNote {
                    title: row.get(0)?,
                    slug: row.get(1)?,
                    relative_path: row.get(2)?,
                    content: row.get(3)?,
                    size_bytes: row.get(4)?,
                    mtime_ns: row.get(5)?,
                    layer: row.get(6)?,
                    metadata: NoteMetadata {
                        tags: Vec::new(),
                        aliases: serde_json::from_str(&row.get::<_, String>(7)?).map_err(
                            |error| {
                                rusqlite::Error::FromSqlConversionFailure(
                                    7,
                                    rusqlite::types::Type::Text,
                                    Box::new(error),
                                )
                            },
                        )?,
                        properties: serde_json::from_str(&row.get::<_, String>(8)?).map_err(
                            |error| {
                                rusqlite::Error::FromSqlConversionFailure(
                                    8,
                                    rusqlite::types::Type::Text,
                                    Box::new(error),
                                )
                            },
                        )?,
                    },
                })
            })
            .map_err(|error| format!("query Vault snapshot notes: {error}"))?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|error| format!("read Vault snapshot notes: {error}"))?;
        drop(notes_statement);

        let mut links_statement = conn
            .prepare(
                "SELECT source_slug, target_slug FROM vault_note_links \
                 WHERE vault_id = ?1 ORDER BY source_slug, target_slug",
            )
            .map_err(|error| format!("prepare Vault snapshot links: {error}"))?;
        let links = links_statement
            .query_map(params![&vault_id], |row| {
                Ok(VaultSnapshotLink {
                    source_slug: row.get(0)?,
                    target_slug: row.get(1)?,
                })
            })
            .map_err(|error| format!("query Vault snapshot links: {error}"))?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|error| format!("read Vault snapshot links: {error}"))?;
        drop(links_statement);

        let mut tags_statement = conn
            .prepare(
                "SELECT note_slug, tag FROM vault_tags WHERE vault_id = ?1 \
                 ORDER BY note_slug, tag",
            )
            .map_err(|error| format!("prepare Vault snapshot tags: {error}"))?;
        let mut tags_by_note = BTreeMap::<String, Vec<String>>::new();
        let tag_rows = tags_statement
            .query_map(params![&vault_id], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(|error| format!("query Vault snapshot tags: {error}"))?;
        for tag in tag_rows {
            let (slug, tag) = tag.map_err(|error| format!("read Vault snapshot tag: {error}"))?;
            tags_by_note.entry(slug).or_default().push(tag);
        }

        for note in &mut notes {
            note.metadata.tags = tags_by_note.get(&note.slug).cloned().unwrap_or_default();
        }
        drop(tags_statement);

        let mut chunks_statement = conn
            .prepare(
                "SELECT c.id, c.note_slug, c.heading_path, c.content, n.layer, \
                 COALESCE(v.embedding, d.embedding) \
                 FROM vault_chunks c \
                 JOIN vault_notes n ON n.vault_id = c.vault_id AND n.slug = c.note_slug \
                 LEFT JOIN vault_chunk_vectors v ON v.chunk_id = c.id \
                 LEFT JOIN vault_chunk_vectors_demoted d ON d.chunk_id = c.id \
                 WHERE c.vault_id = ?1 ORDER BY c.id",
            )
            .map_err(|error| format!("prepare Vault snapshot chunks: {error}"))?;
        let chunks = chunks_statement
            .query_map(params![&vault_id], |row| {
                Ok(VaultSnapshotChunk {
                    chunk_id: row.get(0)?,
                    note_slug: row.get(1)?,
                    heading_path: row.get(2)?,
                    content: row.get(3)?,
                    layer: row.get(4)?,
                    embedding: row.get(5)?,
                })
            })
            .map_err(|error| format!("query Vault snapshot chunks: {error}"))?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|error| format!("read Vault snapshot chunks: {error}"))?;

        let layer_catalog_json: Option<String> = conn
            .query_row(
                "SELECT value FROM vault_snapshot_metadata WHERE vault_id = ?1 AND key = 'layer_catalog'",
                params![&vault_id],
                |row| row.get(0),
            )
            .optional()
            .map_err(|error| format!("read Vault snapshot layer catalog: {error}"))?;
        let layer_catalog = match layer_catalog_json {
            Some(json) => serde_json::from_str(&json)
                .map_err(|error| format!("parse Vault snapshot layer catalog: {error}"))?,
            None => Vec::new(),
        };

        Ok(VaultSnapshotRead {
            notes,
            links,
            tags_by_note,
            chunks,
            layer_catalog,
        })
    }

    fn snapshot_count(
        &self,
        relation: SnapshotRelation,
        vault_id: VaultId,
    ) -> Result<usize, String> {
        let table = relation.table();
        let conn = self.read()?;
        let count = conn
            .query_row(
                &format!("SELECT COUNT(*) FROM {table} WHERE vault_id = ?1"),
                params![vault_id.to_string()],
                |row| row.get::<_, i64>(0),
            )
            .map_err(|error| format!("count Vault snapshot rows in {table}: {error}"))?;
        usize::try_from(count)
            .map_err(|error| format!("count Vault snapshot rows in {table}: {error}"))
    }

    fn begin_vault_snapshot_attempt(&self, vault_id: VaultId) -> Result<u64, String> {
        let mut attempts = self
            .vault_snapshot_attempts
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let attempt = attempts
            .get(&vault_id)
            .copied()
            .unwrap_or_default()
            .checked_add(1)
            .ok_or_else(|| format!("Vault snapshot attempt counter overflowed for {vault_id}"))?;
        attempts.insert(vault_id, attempt);
        Ok(attempt)
    }

    fn mark_vault_snapshot_stale_if_current(
        &self,
        vault_id: VaultId,
        attempt: u64,
    ) -> Result<(), String> {
        let attempts = self
            .vault_snapshot_attempts
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if attempts.get(&vault_id).copied() != Some(attempt) {
            return Ok(());
        }
        let conn = self.connection()?;
        conn.execute(
            "UPDATE vault_snapshots SET freshness = 'stale' WHERE vault_id = ?1",
            params![vault_id.to_string()],
        )
        .map_err(|error| format!("mark Vault snapshot {vault_id} stale: {error}"))?;
        drop(attempts);
        Ok(())
    }

    /// Copy one Vault's published snapshot back into a fresh candidate cache:
    /// the exact inverse of [`SqliteCache::publish_vault_candidate`], and it
    /// must stay in step with it table for table.
    ///
    /// The build path decides what to re-embed by looking up each chunk's
    /// content hash *in the database it is writing to* (`preserve_existing_vectors`
    /// in `cache::populate`), and skips an unchanged note entirely on its
    /// stored hash/mtime/size. A candidate that starts empty therefore misses
    /// every lookup by construction. Seeding restores exactly the state the
    /// single-Vault startup path enjoys on its durable cache, so an unchanged
    /// Vault costs a row copy instead of a full re-embed.
    ///
    /// Reuse is safe here because vectors are keyed by a hash of the embedded
    /// input and `reset_if_embedder_changed` has already run: a model swap
    /// wipes the snapshot before anything can be seeded from it, so no vector
    /// can cross an embedding-space boundary. All work happens in one candidate
    /// transaction, so a partial seed is never visible — either the whole prior
    /// snapshot lands or the candidate stays empty.
    fn seed_candidate_from_snapshot(
        &self,
        vault_id: VaultId,
        candidate: &SqliteCache,
    ) -> Result<(), String> {
        let vault_id = vault_id.to_string();
        let source = self.connection()?;
        // Only a searchable generation is worth seeding. A structure-only one
        // has no vectors to reuse, and seeding it would be actively harmful:
        // its note rows carry current hashes/mtimes, so `upsert_note_if_changed`
        // would report every note `Unchanged`, skip chunking, and leave the
        // Vault permanently without vectors.
        if source
            .query_row(
                "SELECT 1 FROM vault_snapshots WHERE vault_id = ?1 AND searchable = 1",
                params![vault_id],
                |row| row.get::<_, i64>(0),
            )
            .optional()
            .map_err(|error| format!("look up the published Vault snapshot: {error}"))?
            .is_none()
        {
            return Ok(());
        }

        let mut target = candidate.connection()?;
        let tx = target
            .transaction()
            .map_err(|error| format!("start candidate seeding: {error}"))?;

        seed_metadata(&tx, &source, &vault_id)?;
        seed_notes(&tx, &source, &vault_id)?;
        seed_two_string_rows(
            &tx,
            &source,
            &vault_id,
            "SELECT source_slug, target_slug FROM vault_note_links WHERE vault_id = ?1",
            "INSERT INTO note_links(source_slug, target_slug) VALUES (?1, ?2)",
            "snapshot links",
        )?;
        seed_two_string_rows(
            &tx,
            &source,
            &vault_id,
            "SELECT note_slug, tag FROM vault_tags WHERE vault_id = ?1",
            "INSERT INTO tags(note_slug, tag) VALUES (?1, ?2)",
            "snapshot tags",
        )?;
        seed_headings(&tx, &source, &vault_id)?;
        seed_chunks_and_vectors(&tx, &source, &vault_id)?;

        tx.commit()
            .map_err(|error| format!("commit candidate seeding: {error}"))
    }

    /// `searchable` records whether this generation carries vectors. A
    /// structure-only generation publishes with `false`; the embedding pass
    /// that follows republishes the same Vault with `true`.
    fn publish_vault_candidate(
        &self,
        vault_id: VaultId,
        attempt: u64,
        candidate: &SqliteCache,
        embedder_identity: &str,
        searchable: bool,
    ) -> Result<(), String> {
        let source = candidate.read()?;
        let attempts = self
            .vault_snapshot_attempts
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if attempts.get(&vault_id).copied() != Some(attempt) {
            return Ok(());
        }
        let vault_id = vault_id.to_string();
        let mut conn = self.connection()?;
        let tx = conn
            .transaction()
            .map_err(|error| format!("start Vault snapshot publication: {error}"))?;

        delete_vault_snapshot(&tx, &vault_id)?;
        tx.execute(
            "INSERT INTO vault_snapshots(vault_id, participating, freshness, searchable) \
             VALUES (?1, 1, 'fresh', ?2)",
            params![vault_id, i64::from(searchable)],
        )
        .map_err(|error| format!("create Vault snapshot: {error}"))?;

        copy_metadata(&tx, &source, &vault_id)?;
        copy_notes(&tx, &source, &vault_id)?;
        copy_links(&tx, &source, &vault_id)?;
        copy_headings(&tx, &source, &vault_id)?;
        copy_tags(&tx, &source, &vault_id)?;
        copy_chunks_and_vectors(&tx, &source, &vault_id)?;
        tx.execute(
            "INSERT INTO metadata(key, value) VALUES ('embedder_id', ?1) \
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![embedder_identity],
        )
        .map_err(|error| format!("stamp shared embedder identity: {error}"))?;

        let result = tx
            .commit()
            .map_err(|error| format!("commit Vault snapshot publication: {error}"));
        drop(attempts);
        result
    }
}

fn delete_vault_snapshot(tx: &Transaction<'_>, vault_id: &str) -> Result<(), String> {
    tx.execute(
        "DELETE FROM vault_chunk_vectors WHERE chunk_id IN \
         (SELECT id FROM vault_chunks WHERE vault_id = ?1)",
        params![vault_id],
    )
    .map_err(|error| format!("clear Vault default vectors: {error}"))?;
    tx.execute(
        "DELETE FROM vault_chunk_vectors_demoted WHERE chunk_id IN \
         (SELECT id FROM vault_chunks WHERE vault_id = ?1)",
        params![vault_id],
    )
    .map_err(|error| format!("clear Vault demoted vectors: {error}"))?;
    tx.execute(
        "DELETE FROM vault_note_fts WHERE vault_id = ?1",
        params![vault_id],
    )
    .map_err(|error| format!("clear Vault note FTS rows: {error}"))?;
    tx.execute(
        "DELETE FROM vault_snapshots WHERE vault_id = ?1",
        params![vault_id],
    )
    .map_err(|error| format!("clear Vault snapshot rows: {error}"))?;
    Ok(())
}

fn copy_metadata(
    tx: &Transaction<'_>,
    source: &rusqlite::Connection,
    vault_id: &str,
) -> Result<(), String> {
    let mut statement = source
        .prepare("SELECT key, value FROM metadata")
        .map_err(|error| format!("prepare candidate metadata: {error}"))?;
    let rows = statement
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .map_err(|error| format!("query candidate metadata: {error}"))?;
    for row in rows {
        let (key, value) = row.map_err(|error| format!("read candidate metadata: {error}"))?;
        tx.execute(
            "INSERT INTO vault_snapshot_metadata(vault_id, key, value) VALUES (?1, ?2, ?3)",
            params![vault_id, key, value],
        )
        .map_err(|error| format!("copy candidate metadata: {error}"))?;
    }
    Ok(())
}

fn copy_notes(
    tx: &Transaction<'_>,
    source: &rusqlite::Connection,
    vault_id: &str,
) -> Result<(), String> {
    let mut statement = source
        .prepare(
            "SELECT slug, title, normalized_title, relative_path, normalized_relative_path, \
                    absolute_path, content, content_hash, layer, aliases_json, frontmatter_json, \
                    mtime_ns, size_bytes, indexed_at FROM notes",
        )
        .map_err(|error| format!("prepare candidate notes: {error}"))?;
    let rows = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, String>(5)?,
                row.get::<_, String>(6)?,
                row.get::<_, String>(7)?,
                row.get::<_, Option<String>>(8)?,
                row.get::<_, String>(9)?,
                row.get::<_, String>(10)?,
                row.get::<_, i64>(11)?,
                row.get::<_, i64>(12)?,
                row.get::<_, i64>(13)?,
            ))
        })
        .map_err(|error| format!("query candidate notes: {error}"))?;
    for row in rows {
        let (
            slug,
            title,
            normalized_title,
            relative_path,
            normalized_relative_path,
            absolute_path,
            content,
            content_hash,
            layer,
            aliases_json,
            frontmatter_json,
            mtime_ns,
            size_bytes,
            indexed_at,
        ) = row.map_err(|error| format!("read candidate note: {error}"))?;
        tx.execute(
            "INSERT INTO vault_notes(vault_id, slug, title, normalized_title, relative_path, \
             normalized_relative_path, absolute_path, content, content_hash, layer, aliases_json, \
             frontmatter_json, mtime_ns, size_bytes, indexed_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)",
            params![
                vault_id,
                slug,
                title,
                normalized_title,
                relative_path,
                normalized_relative_path,
                absolute_path,
                content,
                content_hash,
                layer,
                aliases_json,
                frontmatter_json,
                mtime_ns,
                size_bytes,
                indexed_at,
            ],
        )
        .map_err(|error| format!("copy candidate note: {error}"))?;
        tx.execute(
            "INSERT INTO vault_note_fts(vault_id, title, relative_path, content, slug) \
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![vault_id, title, relative_path, content, slug],
        )
        .map_err(|error| format!("copy candidate note FTS: {error}"))?;
    }
    Ok(())
}

fn seed_metadata(
    tx: &Transaction<'_>,
    source: &rusqlite::Connection,
    vault_id: &str,
) -> Result<(), String> {
    let mut statement = source
        .prepare("SELECT key, value FROM vault_snapshot_metadata WHERE vault_id = ?1")
        .map_err(|error| format!("prepare snapshot metadata: {error}"))?;
    let rows = statement
        .query_map(params![vault_id], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .map_err(|error| format!("query snapshot metadata: {error}"))?;
    for row in rows {
        let (key, value) = row.map_err(|error| format!("read snapshot metadata: {error}"))?;
        // The build reads back marker-set and embed-layer metadata to decide
        // whether every note needs a forced refresh; a fresh candidate already
        // carries its own schema_version, so replace rather than insert.
        tx.execute(
            "INSERT INTO metadata(key, value) VALUES (?1, ?2) \
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![key, value],
        )
        .map_err(|error| format!("seed snapshot metadata: {error}"))?;
    }
    Ok(())
}

fn seed_notes(
    tx: &Transaction<'_>,
    source: &rusqlite::Connection,
    vault_id: &str,
) -> Result<(), String> {
    let mut statement = source
        .prepare(
            "SELECT slug, title, normalized_title, relative_path, normalized_relative_path, \
                    absolute_path, content, content_hash, layer, aliases_json, frontmatter_json, \
                    mtime_ns, size_bytes, indexed_at FROM vault_notes WHERE vault_id = ?1",
        )
        .map_err(|error| format!("prepare snapshot notes: {error}"))?;
    let rows = statement
        .query_map(params![vault_id], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, String>(5)?,
                row.get::<_, String>(6)?,
                row.get::<_, String>(7)?,
                row.get::<_, Option<String>>(8)?,
                row.get::<_, String>(9)?,
                row.get::<_, String>(10)?,
                row.get::<_, i64>(11)?,
                row.get::<_, i64>(12)?,
                row.get::<_, i64>(13)?,
            ))
        })
        .map_err(|error| format!("query snapshot notes: {error}"))?;
    for row in rows {
        let (
            slug,
            title,
            normalized_title,
            relative_path,
            normalized_relative_path,
            absolute_path,
            content,
            content_hash,
            layer,
            aliases_json,
            frontmatter_json,
            mtime_ns,
            size_bytes,
            indexed_at,
        ) = row.map_err(|error| format!("read snapshot note: {error}"))?;
        let note_id: i64 = tx
            .query_row(
                "INSERT INTO notes(slug, title, normalized_title, relative_path, \
                 normalized_relative_path, absolute_path, content, content_hash, layer, \
                 aliases_json, frontmatter_json, mtime_ns, size_bytes, indexed_at) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14) \
                 RETURNING id",
                params![
                    slug,
                    title,
                    normalized_title,
                    relative_path,
                    normalized_relative_path,
                    absolute_path,
                    content,
                    content_hash,
                    layer,
                    aliases_json,
                    frontmatter_json,
                    mtime_ns,
                    size_bytes,
                    indexed_at,
                ],
                |row| row.get(0),
            )
            .map_err(|error| format!("seed snapshot note: {error}"))?;
        // `notes` has no FTS trigger — the build maintains note_fts by hand, so
        // seeding must too or an unchanged note would publish without its row.
        tx.execute(
            "INSERT INTO note_fts(rowid, title, relative_path, content, slug) \
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![note_id, title, relative_path, content, slug],
        )
        .map_err(|error| format!("seed snapshot note FTS: {error}"))?;
    }
    Ok(())
}

fn seed_headings(
    tx: &Transaction<'_>,
    source: &rusqlite::Connection,
    vault_id: &str,
) -> Result<(), String> {
    let mut statement = source
        .prepare(
            "SELECT note_slug, level, text, anchor, position FROM vault_headings \
             WHERE vault_id = ?1",
        )
        .map_err(|error| format!("prepare snapshot headings: {error}"))?;
    let rows = statement
        .query_map(params![vault_id], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, i64>(4)?,
            ))
        })
        .map_err(|error| format!("query snapshot headings: {error}"))?;
    for row in rows {
        let (note_slug, level, text, anchor, position) =
            row.map_err(|error| format!("read snapshot heading: {error}"))?;
        tx.execute(
            "INSERT INTO headings(note_slug, level, text, anchor, position) \
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![note_slug, level, text, anchor, position],
        )
        .map_err(|error| format!("seed snapshot heading: {error}"))?;
    }
    Ok(())
}

fn seed_two_string_rows(
    tx: &Transaction<'_>,
    source: &rusqlite::Connection,
    vault_id: &str,
    select: &str,
    insert: &str,
    label: &str,
) -> Result<(), String> {
    let mut statement = source
        .prepare(select)
        .map_err(|error| format!("prepare {label}: {error}"))?;
    let rows = statement
        .query_map(params![vault_id], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .map_err(|error| format!("query {label}: {error}"))?;
    for row in rows {
        let (first, second) = row.map_err(|error| format!("read {label}: {error}"))?;
        tx.execute(insert, params![first, second])
            .map_err(|error| format!("seed {label}: {error}"))?;
    }
    Ok(())
}

fn seed_chunks_and_vectors(
    tx: &Transaction<'_>,
    source: &rusqlite::Connection,
    vault_id: &str,
) -> Result<(), String> {
    let mut statement = source
        .prepare(
            "SELECT c.note_slug, c.ordinal, c.heading_path, c.content, c.byte_start, \
                    c.byte_end, c.content_hash, c.tags, c.aliases, v.embedding, \
                    d.embedding, d.layer \
             FROM vault_chunks c \
             LEFT JOIN vault_chunk_vectors v ON v.chunk_id = c.id \
             LEFT JOIN vault_chunk_vectors_demoted d ON d.chunk_id = c.id \
             WHERE c.vault_id = ?1",
        )
        .map_err(|error| format!("prepare snapshot chunks: {error}"))?;
    let rows = statement
        .query_map(params![vault_id], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, Option<String>>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, i64>(4)?,
                row.get::<_, i64>(5)?,
                row.get::<_, String>(6)?,
                row.get::<_, Option<String>>(7)?,
                row.get::<_, Option<String>>(8)?,
                row.get::<_, Option<Vec<u8>>>(9)?,
                row.get::<_, Option<Vec<u8>>>(10)?,
                row.get::<_, Option<String>>(11)?,
            ))
        })
        .map_err(|error| format!("query snapshot chunks: {error}"))?;
    for row in rows {
        let (
            note_slug,
            ordinal,
            heading_path,
            content,
            byte_start,
            byte_end,
            content_hash,
            tags,
            aliases,
            default_embedding,
            demoted_embedding,
            demoted_layer,
        ) = row.map_err(|error| format!("read snapshot chunk: {error}"))?;
        let chunk_id: i64 = tx
            .query_row(
                "INSERT INTO chunks(note_slug, ordinal, heading_path, content, byte_start, \
                 byte_end, content_hash, tags, aliases) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9) RETURNING id",
                params![
                    note_slug,
                    ordinal,
                    heading_path,
                    content,
                    byte_start,
                    byte_end,
                    content_hash,
                    tags,
                    aliases,
                ],
                |row| row.get(0),
            )
            .map_err(|error| format!("seed snapshot chunk: {error}"))?;
        if let Some(embedding) = default_embedding {
            tx.execute(
                "INSERT INTO chunk_vectors(chunk_id, embedding) VALUES (?1, ?2)",
                params![chunk_id, embedding],
            )
            .map_err(|error| format!("seed snapshot default vector: {error}"))?;
        }
        if let Some(embedding) = demoted_embedding {
            tx.execute(
                "INSERT INTO chunk_vectors_demoted(chunk_id, embedding, layer) \
                 VALUES (?1, ?2, ?3)",
                params![chunk_id, embedding, demoted_layer],
            )
            .map_err(|error| format!("seed snapshot demoted vector: {error}"))?;
        }
    }
    Ok(())
}

fn copy_links(
    tx: &Transaction<'_>,
    source: &rusqlite::Connection,
    vault_id: &str,
) -> Result<(), String> {
    copy_two_string_rows(
        tx,
        source,
        vault_id,
        "SELECT source_slug, target_slug FROM note_links",
        "INSERT INTO vault_note_links(vault_id, source_slug, target_slug) VALUES (?1, ?2, ?3)",
        "candidate links",
    )
}

fn copy_tags(
    tx: &Transaction<'_>,
    source: &rusqlite::Connection,
    vault_id: &str,
) -> Result<(), String> {
    copy_two_string_rows(
        tx,
        source,
        vault_id,
        "SELECT note_slug, tag FROM tags",
        "INSERT INTO vault_tags(vault_id, note_slug, tag) VALUES (?1, ?2, ?3)",
        "candidate tags",
    )
}

fn copy_two_string_rows(
    tx: &Transaction<'_>,
    source: &rusqlite::Connection,
    vault_id: &str,
    select: &str,
    insert: &str,
    label: &str,
) -> Result<(), String> {
    let mut statement = source
        .prepare(select)
        .map_err(|error| format!("prepare {label}: {error}"))?;
    let rows = statement
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .map_err(|error| format!("query {label}: {error}"))?;
    for row in rows {
        let (first, second) = row.map_err(|error| format!("read {label}: {error}"))?;
        tx.execute(insert, params![vault_id, first, second])
            .map_err(|error| format!("copy {label}: {error}"))?;
    }
    Ok(())
}

fn copy_headings(
    tx: &Transaction<'_>,
    source: &rusqlite::Connection,
    vault_id: &str,
) -> Result<(), String> {
    let mut statement = source
        .prepare("SELECT note_slug, level, text, anchor, position FROM headings")
        .map_err(|error| format!("prepare candidate headings: {error}"))?;
    let rows = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, i64>(4)?,
            ))
        })
        .map_err(|error| format!("query candidate headings: {error}"))?;
    for row in rows {
        let (note_slug, level, text, anchor, position) =
            row.map_err(|error| format!("read candidate heading: {error}"))?;
        tx.execute(
            "INSERT INTO vault_headings(vault_id, note_slug, level, text, anchor, position) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![vault_id, note_slug, level, text, anchor, position],
        )
        .map_err(|error| format!("copy candidate heading: {error}"))?;
    }
    Ok(())
}

fn copy_chunks_and_vectors(
    tx: &Transaction<'_>,
    source: &rusqlite::Connection,
    vault_id: &str,
) -> Result<(), String> {
    let mut statement = source
        .prepare(
            "SELECT c.id, c.note_slug, c.ordinal, c.heading_path, c.content, c.byte_start, \
                    c.byte_end, c.content_hash, c.tags, c.aliases, v.embedding, \
                    d.embedding, d.layer \
             FROM chunks c \
             LEFT JOIN chunk_vectors v ON v.chunk_id = c.id \
             LEFT JOIN chunk_vectors_demoted d ON d.chunk_id = c.id",
        )
        .map_err(|error| format!("prepare candidate chunks: {error}"))?;
    let rows = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, Option<String>>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, i64>(5)?,
                row.get::<_, i64>(6)?,
                row.get::<_, String>(7)?,
                row.get::<_, Option<String>>(8)?,
                row.get::<_, Option<String>>(9)?,
                row.get::<_, Option<Vec<u8>>>(10)?,
                row.get::<_, Option<Vec<u8>>>(11)?,
                row.get::<_, Option<String>>(12)?,
            ))
        })
        .map_err(|error| format!("query candidate chunks: {error}"))?;
    for row in rows {
        let (
            _candidate_id,
            note_slug,
            ordinal,
            heading_path,
            content,
            byte_start,
            byte_end,
            content_hash,
            tags,
            aliases,
            default_embedding,
            demoted_embedding,
            demoted_layer,
        ) = row.map_err(|error| format!("read candidate chunk: {error}"))?;
        let chunk_id: i64 = tx
            .query_row(
                "INSERT INTO vault_chunks(vault_id, note_slug, ordinal, heading_path, content, \
                 byte_start, byte_end, content_hash, tags, aliases) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10) RETURNING id",
                params![
                    vault_id,
                    note_slug,
                    ordinal,
                    heading_path,
                    content,
                    byte_start,
                    byte_end,
                    content_hash,
                    tags,
                    aliases,
                ],
                |row| row.get(0),
            )
            .map_err(|error| format!("copy candidate chunk: {error}"))?;
        if let Some(embedding) = default_embedding {
            tx.execute(
                "INSERT INTO vault_chunk_vectors(chunk_id, embedding, vault_id) VALUES (?1, ?2, ?3)",
                params![chunk_id, embedding, vault_id],
            )
            .map_err(|error| format!("copy candidate default vector: {error}"))?;
        }
        if let Some(embedding) = demoted_embedding {
            tx.execute(
                "INSERT INTO vault_chunk_vectors_demoted(chunk_id, embedding, layer, vault_id) \
                 VALUES (?1, ?2, ?3, ?4)",
                params![chunk_id, embedding, demoted_layer, vault_id],
            )
            .map_err(|error| format!("copy candidate demoted vector: {error}"))?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;
    use std::sync::{Arc, Barrier};

    use tempfile::tempdir;

    use super::{VaultSnapshotFreshness, VaultSnapshotStatus};
    use crate::cache::SqliteCache;
    use crate::embed::{Embedder, StubEmbedder};
    use crate::vault::VaultIndex;
    use crate::vault_registry::VaultId;

    struct NamedEmbedder {
        inner: StubEmbedder,
        identity: &'static str,
    }
    impl Embedder for NamedEmbedder {
        fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, String> {
            self.inner.embed(texts)
        }
        fn embedding_dim(&self) -> usize {
            384
        }
        fn identity(&self) -> String {
            self.identity.to_string()
        }
        fn token_count(&self, text: &str, add: bool) -> Result<usize, String> {
            self.inner.token_count(text, add)
        }
    }

    /// Counts the texts actually sent for embedding, so a test can prove that a
    /// rebuild reused prior vectors instead of paying for them again.
    struct CountingEmbedder {
        inner: StubEmbedder,
        embedded: std::sync::atomic::AtomicUsize,
    }
    impl CountingEmbedder {
        fn new() -> Self {
            Self {
                inner: StubEmbedder::new(384),
                embedded: std::sync::atomic::AtomicUsize::new(0),
            }
        }
        fn embedded(&self) -> usize {
            self.embedded.load(std::sync::atomic::Ordering::Relaxed)
        }
        fn reset(&self) {
            self.embedded.store(0, std::sync::atomic::Ordering::Relaxed);
        }
    }
    impl Embedder for CountingEmbedder {
        fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, String> {
            self.embedded
                .fetch_add(texts.len(), std::sync::atomic::Ordering::Relaxed);
            self.inner.embed(texts)
        }
        fn embedding_dim(&self) -> usize {
            384
        }
        fn identity(&self) -> String {
            "counting-384".to_string()
        }
        fn token_count(&self, text: &str, add: bool) -> Result<usize, String> {
            self.inner.token_count(text, add)
        }
    }

    fn vault_id(value: &str) -> VaultId {
        VaultId::from_str(value).expect("known Vault ID")
    }

    fn index(files: &[(&str, &str)]) -> (tempfile::TempDir, VaultIndex) {
        let directory = tempdir().expect("tempdir");
        for (path, content) in files {
            let path = directory.path().join(path);
            std::fs::create_dir_all(path.parent().expect("parent")).expect("create parent");
            std::fs::write(path, content).expect("write note");
        }
        let index = VaultIndex::build(directory.path()).expect("build index");
        (directory, index)
    }

    /// How many of a published generation's chunks carry a vector. Zero is the
    /// structure-only signature; search skips vectorless chunks, so this is
    /// exactly what separates a browsable Vault from a searchable one.
    fn vectored_chunks(cache: &SqliteCache, vault_id: VaultId) -> usize {
        cache
            .read_vault_snapshot(vault_id)
            .expect("read published snapshot")
            .expect("snapshot participates")
            .read
            .chunks
            .iter()
            .filter(|chunk| chunk.embedding.is_some())
            .count()
    }

    #[test]
    fn structure_pass_publishes_browsable_rows_with_no_vectors() {
        let cache = SqliteCache::in_memory(384).expect("open cache");
        let id = vault_id("12345678-1234-4567-89ab-1234567890ab");
        let (_directory, index) = index(&[
            ("Home.md", "# Home\n\nhome body\n\n[[Shared]]"),
            ("Shared.md", "# Shared\n\nshared body"),
        ]);
        let embedder = CountingEmbedder::new();

        let published = cache
            .publish_vault_structure_snapshot(id, &index, &embedder, true)
            .expect("publish structure-only snapshot");

        assert!(published, "a Vault with no prior generation publishes one");
        assert_eq!(
            embedder.embedded(),
            0,
            "the structure pass must not embed anything: that is the wait it exists to remove"
        );
        assert_eq!(
            cache.snapshot_status(id).expect("read status"),
            Some(VaultSnapshotStatus {
                participating: true,
                freshness: VaultSnapshotFreshness::Fresh,
                searchable: false,
            }),
            "structural rows are current, so the generation is fresh but not searchable"
        );
        let read = cache
            .read_vault_snapshot(id)
            .expect("read snapshot")
            .expect("snapshot participates")
            .read;
        assert_eq!(read.notes.len(), 2, "browsing has every Note");
        assert_eq!(read.links.len(), 1, "and the links between them");
        assert_eq!(vectored_chunks(&cache, id), 0);
    }

    /// The trap this design has to clear: the structure pass writes note rows
    /// carrying current hashes and mtimes. If the embedding pass seeded its
    /// candidate from them, every note would report `Unchanged`, chunking would
    /// be skipped, and the Vault would stay unsearchable forever.
    #[test]
    fn embedding_pass_after_a_structure_pass_still_vectors_every_chunk() {
        let cache = SqliteCache::in_memory(384).expect("open cache");
        let id = vault_id("12345678-1234-4567-89ab-1234567890ab");
        let (_directory, index) = index(&[
            ("Home.md", "# Home\n\nhome body\n\n[[Shared]]"),
            ("Shared.md", "# Shared\n\nshared body"),
        ]);
        let embedder = CountingEmbedder::new();

        cache
            .publish_vault_structure_snapshot(id, &index, &embedder, true)
            .expect("publish structure-only snapshot");
        assert_eq!(vectored_chunks(&cache, id), 0);

        cache
            .replace_vault_snapshot(id, &index, &embedder)
            .expect("publish the searchable generation");

        assert!(
            embedder.embedded() > 0,
            "the embedding pass must not be short-circuited by the structure pass's note rows"
        );
        assert_eq!(
            cache.snapshot_status(id).expect("read status"),
            Some(VaultSnapshotStatus {
                participating: true,
                freshness: VaultSnapshotFreshness::Fresh,
                searchable: true,
            })
        );
        assert_eq!(
            vectored_chunks(&cache, id),
            2,
            "every chunk carries a vector once the embedding pass has run"
        );
    }

    /// Publishing structure over an already-searchable Vault would take working
    /// search away for the length of every rebuild. Serving the prior
    /// generation marked stale is strictly better, so the structure pass only
    /// ever fires on a Vault's first index.
    #[test]
    fn structure_pass_is_a_no_op_once_a_vault_is_searchable() {
        let cache = SqliteCache::in_memory(384).expect("open cache");
        let id = vault_id("12345678-1234-4567-89ab-1234567890ab");
        let (_directory, index) = index(&[("Home.md", "# Home\n\nhome body")]);
        let embedder = CountingEmbedder::new();

        cache
            .replace_vault_snapshot(id, &index, &embedder)
            .expect("publish the searchable generation");
        let vectored = vectored_chunks(&cache, id);
        assert!(vectored > 0);

        let published = cache
            .publish_vault_structure_snapshot(id, &index, &embedder, true)
            .expect("structure pass runs without error");

        assert!(!published, "an already-searchable Vault publishes nothing");
        assert_eq!(
            cache.snapshot_status(id).expect("read status"),
            Some(VaultSnapshotStatus {
                participating: true,
                freshness: VaultSnapshotFreshness::Fresh,
                searchable: true,
            }),
            "search stays available across a rebuild"
        );
        assert_eq!(vectored_chunks(&cache, id), vectored);
    }

    /// Regression: retirement keeps a disabled Vault's rows and only clears
    /// participation, so its vectors survive. Re-enabling must not fire the
    /// structure pass over them — that publication deletes every retained
    /// vector, and the embedding pass that follows then has nothing to seed
    /// from, turning a row copy into a full re-embed of the whole Vault.
    #[test]
    fn re_enabling_a_disabled_vault_reuses_its_retained_vectors() {
        let cache = SqliteCache::in_memory(384).expect("open cache");
        let id = vault_id("12345678-1234-4567-89ab-1234567890ab");
        let (_directory, index) = index(&[
            ("Home.md", "# Home\n\nhome body"),
            ("Shared.md", "# Shared\n\nshared body"),
        ]);
        let embedder = CountingEmbedder::new();

        cache
            .replace_vault_snapshot(id, &index, &embedder)
            .expect("publish the searchable generation");
        assert!(embedder.embedded() > 0);
        cache
            .disable_vault_snapshot(id)
            .expect("retire the Vault without deleting its rows");

        // Re-enable: reconciliation clears participation again, then queues an
        // Index turn, which runs both passes.
        cache
            .disable_vault_snapshot(id)
            .expect("re-enable clears participation first");
        embedder.reset();
        let published = cache
            .publish_vault_structure_snapshot(id, &index, &embedder, true)
            .expect("structure pass runs without error");
        assert!(
            !published,
            "a disabled Vault's retained generation is still searchable, so the \
             structure pass must not replace it"
        );
        cache
            .replace_vault_snapshot(id, &index, &embedder)
            .expect("republish on re-enable");

        assert_eq!(
            embedder.embedded(),
            0,
            "re-enabling an unchanged Vault must reuse its retained vectors, not re-embed it"
        );
        assert_eq!(vectored_chunks(&cache, id), 2);
    }

    /// Regression: every Index turn built its candidate in a fresh empty cache,
    /// so the build's content-hash reuse lookup could never hit and an
    /// unchanged Vault re-embedded itself in full — on every edit and every
    /// restart.
    #[test]
    fn rebuilding_an_unchanged_vault_reuses_its_published_vectors() {
        let cache = SqliteCache::in_memory(384).expect("open cache");
        let id = vault_id("12345678-1234-4567-89ab-1234567890ab");
        let (_directory, index) = index(&[
            ("Home.md", "# Home\n\nhome body\n\n[[Shared]]"),
            ("Shared.md", "# Shared\n\nshared body"),
        ]);
        let embedder = CountingEmbedder::new();

        cache
            .replace_vault_snapshot(id, &index, &embedder)
            .expect("publish the first snapshot");
        let first_pass = embedder.embedded();
        assert!(first_pass > 0, "the first build must embed something");

        embedder.reset();
        cache
            .replace_vault_snapshot(id, &index, &embedder)
            .expect("rebuild the unchanged Vault");
        assert_eq!(
            embedder.embedded(),
            0,
            "an unchanged Vault must reuse every published vector"
        );

        // Reuse must not cost the snapshot any content.
        assert_eq!(cache.snapshot_note_count(id).expect("notes"), 2);
        assert_eq!(cache.snapshot_chunk_count(id).expect("chunks"), 2);
        assert_eq!(cache.snapshot_link_count(id).expect("links"), 1);
        assert_eq!(
            cache
                .snapshot_note_content(id, "home")
                .expect("content")
                .as_deref(),
            Some("# Home\n\nhome body\n\n[[Shared]]")
        );
    }

    #[test]
    fn editing_one_note_re_embeds_only_that_note() {
        let cache = SqliteCache::in_memory(384).expect("open cache");
        let id = vault_id("12345678-1234-4567-89ab-1234567890ab");
        let (directory, first_index) = index(&[
            ("Home.md", "# Home\n\nhome body"),
            ("Shared.md", "# Shared\n\nshared body"),
        ]);
        let embedder = CountingEmbedder::new();
        cache
            .replace_vault_snapshot(id, &first_index, &embedder)
            .expect("publish the first snapshot");

        std::fs::write(
            directory.path().join("Home.md"),
            "# Home\n\nhome body, rewritten",
        )
        .expect("edit one note");
        let edited = VaultIndex::build(directory.path()).expect("rebuild index");
        embedder.reset();
        cache
            .replace_vault_snapshot(id, &edited, &embedder)
            .expect("rebuild after the edit");

        assert_eq!(
            embedder.embedded(),
            1,
            "only the edited note's chunk may be re-embedded"
        );
        assert_eq!(
            cache
                .snapshot_note_content(id, "home")
                .expect("content")
                .as_deref(),
            Some("# Home\n\nhome body, rewritten"),
            "the edit must still reach the published snapshot"
        );
    }

    /// Reuse must never cross an embedding space: a different model wipes the
    /// shared cache before seeding can offer it anything.
    #[test]
    fn a_changed_embedder_re_embeds_instead_of_reusing_seeded_vectors() {
        let cache = SqliteCache::in_memory(384).expect("open cache");
        let id = vault_id("12345678-1234-4567-89ab-1234567890ab");
        let (_directory, index) = index(&[("Home.md", "# Home\n\nhome body")]);

        cache
            .replace_vault_snapshot(
                id,
                &index,
                &NamedEmbedder {
                    inner: StubEmbedder::new(384),
                    identity: "first-model-384",
                },
            )
            .expect("publish with the first model");

        let second = CountingEmbedder::new();
        cache
            .replace_vault_snapshot(id, &index, &second)
            .expect("rebuild with a different model");
        assert!(
            second.embedded() > 0,
            "a model swap must re-embed rather than reuse the previous space"
        );
    }

    #[test]
    fn equal_note_identities_and_links_remain_isolated_by_vault_id() {
        let cache = SqliteCache::in_memory(384).expect("open cache");
        let first = vault_id("12345678-1234-4567-89ab-1234567890ab");
        let second = vault_id("22345678-1234-4567-89ab-1234567890ab");
        let (_first_directory, first_index) = index(&[
            ("Home.md", "# Home\n\nfirst-only chunk\n\n[[Shared]]"),
            ("Shared.md", "# Shared\n\nfirst target"),
        ]);
        let (_second_directory, second_index) = index(&[
            ("Home.md", "# Home\n\nsecond-only chunk\n\n[[Shared]]"),
            ("Shared.md", "# Shared\n\nsecond target"),
        ]);
        let embedder = StubEmbedder::new(384);

        cache
            .replace_vault_snapshot(first, &first_index, &embedder)
            .expect("publish first Vault snapshot");
        cache
            .replace_vault_snapshot(second, &second_index, &embedder)
            .expect("publish second Vault snapshot");

        assert_eq!(cache.snapshot_note_count(first).expect("first notes"), 2);
        assert_eq!(cache.snapshot_note_count(second).expect("second notes"), 2);
        assert_eq!(cache.snapshot_link_count(first).expect("first links"), 1);
        assert_eq!(cache.snapshot_link_count(second).expect("second links"), 1);
        assert_eq!(cache.snapshot_chunk_count(first).expect("first chunks"), 2);
        assert_eq!(
            cache.snapshot_chunk_count(second).expect("second chunks"),
            2
        );
        assert_eq!(
            cache
                .snapshot_note_content(first, "home")
                .expect("first content")
                .as_deref(),
            Some("# Home\n\nfirst-only chunk\n\n[[Shared]]")
        );
        assert_eq!(
            cache
                .snapshot_note_content(second, "home")
                .expect("second content")
                .as_deref(),
            Some("# Home\n\nsecond-only chunk\n\n[[Shared]]")
        );
    }

    #[test]
    fn failed_reindex_keeps_the_prior_snapshot_stale_until_a_full_success_reenables_it() {
        let cache = SqliteCache::in_memory(384).expect("open cache");
        let vault_id = vault_id("12345678-1234-4567-89ab-1234567890ab");
        let (_first_directory, first_index) = index(&[("Home.md", "# Home\n\nfirst")]);
        let (second_directory, second_index) = index(&[("Home.md", "# Home\n\nsecond")]);
        let working = StubEmbedder::new(384);

        cache
            .replace_vault_snapshot(vault_id, &first_index, &working)
            .expect("publish first snapshot");
        std::fs::remove_file(second_directory.path().join("Home.md"))
            .expect("make the pending reindex unreadable");
        let failed = cache.replace_vault_snapshot(vault_id, &second_index, &working);
        assert!(failed.is_err(), "the reindex must report its failure");
        assert_eq!(
            cache
                .snapshot_note_content(vault_id, "home")
                .expect("read retained content")
                .as_deref(),
            Some("# Home\n\nfirst")
        );
        assert_eq!(
            cache.snapshot_status(vault_id).expect("read status"),
            Some(VaultSnapshotStatus {
                participating: true,
                freshness: VaultSnapshotFreshness::Stale,
                searchable: true,
            })
        );

        cache
            .disable_vault_snapshot(vault_id)
            .expect("disable snapshot participation");
        assert_eq!(
            cache
                .snapshot_status(vault_id)
                .expect("read disabled status"),
            Some(VaultSnapshotStatus {
                participating: false,
                freshness: VaultSnapshotFreshness::Stale,
                searchable: true,
            })
        );

        std::fs::write(second_directory.path().join("Home.md"), "# Home\n\nsecond")
            .expect("restore the next full index source");
        cache
            .replace_vault_snapshot(vault_id, &second_index, &working)
            .expect("full reindex reenables the snapshot");
        assert_eq!(
            cache.snapshot_status(vault_id).expect("read fresh status"),
            Some(VaultSnapshotStatus {
                participating: true,
                freshness: VaultSnapshotFreshness::Fresh,
                searchable: true,
            })
        );
        assert_eq!(
            cache
                .snapshot_note_content(vault_id, "home")
                .expect("read replacement content")
                .as_deref(),
            Some("# Home\n\nsecond")
        );
    }

    #[test]
    fn one_vault_publication_is_atomic_for_readers_and_disconnect_is_vault_local() {
        let directory = tempdir().expect("tempdir");
        let cache = SqliteCache::open(directory.path().join("cache.sqlite3"), 384)
            .expect("open file cache");
        let first = vault_id("12345678-1234-4567-89ab-1234567890ab");
        let second = vault_id("22345678-1234-4567-89ab-1234567890ab");
        let (_old_directory, old_index) = index(&[("Home.md", "# Home\n\nold")]);
        let (_new_directory, new_index) = index(&[("Home.md", "# Home\n\nnew")]);
        let (_second_directory, second_index) = index(&[("Home.md", "# Home\n\nsecond")]);
        let embedder = StubEmbedder::new(384);

        cache
            .replace_vault_snapshot(first, &old_index, &embedder)
            .expect("publish first Vault's old snapshot");
        cache
            .replace_vault_snapshot(second, &second_index, &embedder)
            .expect("publish second Vault snapshot");

        let reader = cache.read().expect("checkout reader");
        reader
            .execute_batch("BEGIN")
            .expect("begin reader snapshot");
        let before: String = reader
            .query_row(
                "SELECT content FROM vault_notes WHERE vault_id = ?1 AND slug = 'home'",
                [first.to_string()],
                |row| row.get(0),
            )
            .expect("read old snapshot");
        assert_eq!(before, "# Home\n\nold");

        cache
            .replace_vault_snapshot(first, &new_index, &embedder)
            .expect("replace first Vault only");
        let held: String = reader
            .query_row(
                "SELECT content FROM vault_notes WHERE vault_id = ?1 AND slug = 'home'",
                [first.to_string()],
                |row| row.get(0),
            )
            .expect("reader retains a complete old snapshot");
        assert_eq!(held, "# Home\n\nold");
        reader
            .execute_batch("COMMIT")
            .expect("release reader snapshot");
        drop(reader);

        assert_eq!(
            cache
                .snapshot_note_content(first, "home")
                .expect("read replacement")
                .as_deref(),
            Some("# Home\n\nnew")
        );
        cache
            .disconnect_vault_snapshot(first)
            .expect("disconnect only first Vault");
        assert_eq!(cache.snapshot_status(first).expect("first status"), None);
        assert!(
            cache
                .read_vault_snapshot(first)
                .expect("read disconnected snapshot")
                .is_none(),
            "a disconnected Vault must be unavailable, never an empty projection"
        );
        assert_eq!(cache.snapshot_note_count(first).expect("first rows"), 0);
        assert_eq!(
            cache
                .snapshot_note_content(second, "home")
                .expect("second snapshot remains")
                .as_deref(),
            Some("# Home\n\nsecond")
        );
    }

    #[test]
    fn older_failed_attempt_cannot_stale_a_newer_published_snapshot() {
        let cache = SqliteCache::in_memory(384).expect("open cache");
        let vault_id = vault_id("12345678-1234-4567-89ab-1234567890ab");
        let (_directory, index) = index(&[("Home.md", "# Home\n\nfresh")]);
        let embedder = StubEmbedder::new(384);
        cache
            .replace_vault_snapshot(vault_id, &index, &embedder)
            .expect("publish fresh snapshot");

        let older = cache
            .begin_vault_snapshot_attempt(vault_id)
            .expect("begin older attempt");
        let _newer = cache
            .begin_vault_snapshot_attempt(vault_id)
            .expect("begin newer attempt");
        cache
            .mark_vault_snapshot_stale_if_current(vault_id, older)
            .expect("finish older failed attempt");

        assert_eq!(
            cache.snapshot_status(vault_id).expect("read status"),
            Some(VaultSnapshotStatus {
                participating: true,
                freshness: VaultSnapshotFreshness::Fresh,
                searchable: true,
            })
        );
    }

    #[test]
    fn model_change_wipes_other_vault_snapshots_before_partial_rebuild() {
        let cache = SqliteCache::in_memory(384).expect("open cache");
        let first = vault_id("12345678-1234-4567-89ab-1234567890ab");
        let second = vault_id("22345678-1234-4567-89ab-1234567890ab");
        let (_first_directory, first_index) = index(&[("Home.md", "# Home\n\nfirst")]);
        let (_second_directory, second_index) = index(&[("Home.md", "# Home\n\nsecond")]);
        let old_model = StubEmbedder::new(384);
        let new_model = StubEmbedder::new(512);

        cache
            .replace_vault_snapshot(first, &first_index, &old_model)
            .expect("publish first old-model snapshot");
        cache
            .replace_vault_snapshot(second, &second_index, &old_model)
            .expect("publish second old-model snapshot");
        cache
            .replace_vault_snapshot(first, &first_index, &new_model)
            .expect("publish first rebuilt snapshot");

        assert!(
            cache
                .snapshot_status(first)
                .expect("first status")
                .expect("first rebuilt snapshot")
                .participating
        );
        assert_eq!(
            cache.snapshot_status(second).expect("second status"),
            None,
            "a partial rebuild must not leave old-model vectors participating"
        );
        assert_eq!(
            cache
                .get_metadata("embedder_id")
                .expect("embedder identity"),
            Some("stub-512".to_string())
        );
    }

    #[test]
    fn concurrent_same_dimension_model_changes_leave_one_shared_identity() {
        let cache = Arc::new(SqliteCache::in_memory(384).expect("cache"));
        let first = vault_id("12345678-1234-4567-89ab-1234567890ab");
        let second = vault_id("22345678-1234-4567-89ab-1234567890ab");
        let (_a, first_index) = index(&[("Home.md", "# First")]);
        let (_b, second_index) = index(&[("Home.md", "# Second")]);
        let barrier = Arc::new(Barrier::new(2));
        std::thread::scope(|scope| {
            for (id, index, name) in [
                (first, &first_index, "model-a-384"),
                (second, &second_index, "model-b-384"),
            ] {
                let cache = cache.clone();
                let barrier = barrier.clone();
                scope.spawn(move || {
                    barrier.wait();
                    cache
                        .replace_vault_snapshot(
                            id,
                            index,
                            &NamedEmbedder {
                                inner: StubEmbedder::new(384),
                                identity: name,
                            },
                        )
                        .expect("publish");
                });
            }
        });
        let global = cache
            .get_metadata("embedder_id")
            .expect("global")
            .expect("identity");
        let conn = cache.connection().expect("conn");
        let mut stmt = conn
            .prepare("SELECT value FROM vault_snapshot_metadata WHERE key = 'embedder_id'")
            .expect("metadata");
        let identities = stmt
            .query_map([], |row| row.get::<_, String>(0))
            .expect("query")
            .collect::<rusqlite::Result<Vec<_>>>()
            .expect("rows");
        assert!(!identities.is_empty());
        assert!(identities.iter().all(|identity| identity == &global));
    }

    #[test]
    fn snapshot_and_production_populate_share_one_model_epoch() {
        let cache = Arc::new(SqliteCache::in_memory(384).expect("cache"));
        let vault = vault_id("12345678-1234-4567-89ab-1234567890ab");
        let (_a, snapshot_index) = index(&[("Snapshot.md", "# Snapshot")]);
        let (_b, legacy_index) = index(&[("Legacy.md", "# Legacy")]);
        let barrier = Arc::new(Barrier::new(2));
        std::thread::scope(|scope| {
            let cache_a = cache.clone();
            let barrier_a = barrier.clone();
            scope.spawn(move || {
                barrier_a.wait();
                cache_a
                    .replace_vault_snapshot(
                        vault,
                        &snapshot_index,
                        &NamedEmbedder {
                            inner: StubEmbedder::new(384),
                            identity: "model-a-384",
                        },
                    )
                    .expect("snapshot");
            });
            let cache_b = cache.clone();
            let barrier_b = barrier.clone();
            scope.spawn(move || {
                barrier_b.wait();
                cache_b
                    .replace_from_index_with_progress(
                        &legacy_index,
                        &NamedEmbedder {
                            inner: StubEmbedder::new(384),
                            identity: "model-b-384",
                        },
                        None,
                        true,
                    )
                    .expect("legacy");
            });
        });
        let global = cache
            .get_metadata("embedder_id")
            .expect("global")
            .expect("identity");
        let conn = cache.connection().expect("conn");
        let mut statement = conn
            .prepare("SELECT value FROM vault_snapshot_metadata WHERE key = 'embedder_id'")
            .expect("metadata");
        let identities = statement
            .query_map([], |row| row.get::<_, String>(0))
            .expect("query")
            .collect::<rusqlite::Result<Vec<_>>>()
            .expect("rows");
        assert!(identities.iter().all(|identity| identity == &global));
    }

    #[test]
    fn missing_global_identity_uses_vault_metadata_before_model_change() {
        let cache = SqliteCache::in_memory(384).expect("cache");
        let first = vault_id("12345678-1234-4567-89ab-1234567890ab");
        let second = vault_id("22345678-1234-4567-89ab-1234567890ab");
        let (_a, first_index) = index(&[("Home.md", "# First")]);
        let (_b, second_index) = index(&[("Home.md", "# Second")]);
        let old = NamedEmbedder {
            inner: StubEmbedder::new(384),
            identity: "model-old-384",
        };
        let new = NamedEmbedder {
            inner: StubEmbedder::new(384),
            identity: "model-new-384",
        };
        cache
            .replace_vault_snapshot(first, &first_index, &old)
            .expect("old");
        cache
            .connection()
            .expect("conn")
            .execute("DELETE FROM metadata WHERE key = 'embedder_id'", [])
            .expect("drop global stamp");
        cache
            .replace_vault_snapshot(second, &second_index, &new)
            .expect("new");
        assert_eq!(cache.snapshot_status(first).expect("first"), None);
        assert_eq!(
            cache.get_metadata("embedder_id").expect("global"),
            Some("model-new-384".to_string())
        );
    }

    #[test]
    fn failing_global_identity_stamp_rolls_back_target_publication() {
        let cache = SqliteCache::in_memory(384).expect("cache");
        let first = vault_id("12345678-1234-4567-89ab-1234567890ab");
        let second = vault_id("22345678-1234-4567-89ab-1234567890ab");
        let (_a, first_index) = index(&[("Home.md", "# First")]);
        let (_b, second_index) = index(&[("Home.md", "# Second")]);
        let embedder = StubEmbedder::new(384);
        cache
            .replace_vault_snapshot(first, &first_index, &embedder)
            .expect("first");
        cache.connection().expect("conn").execute_batch("CREATE TRIGGER fail_embedder_stamp BEFORE INSERT ON metadata WHEN NEW.key = 'embedder_id' BEGIN SELECT RAISE(ABORT, 'stamp failed'); END;").expect("trigger");
        assert!(
            cache
                .replace_vault_snapshot(second, &second_index, &embedder)
                .is_err()
        );
        assert_eq!(cache.snapshot_status(second).expect("second"), None);
        assert!(
            cache
                .snapshot_status(first)
                .expect("first")
                .expect("snapshot")
                .participating
        );
        assert_eq!(
            cache.get_metadata("embedder_id").expect("global"),
            Some("stub-384".to_string())
        );
    }
}
