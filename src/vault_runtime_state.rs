//! Durable per-Vault operational state: the facts Hatchdoor must remember
//! about a Vault *between* processes, as opposed to its authoritative
//! configuration (`vault_registry`) or its disposable read model (`cache`).
//!
//! Today that is exactly one fact — when each Vault's last interval-arming
//! Git turn completed, and how it went — which is what lets
//! `git::ManagedGitScheduler` keep a Vault's configured poll interval across
//! a restart instead of restarting the countdown every time the process
//! does. The record is nested under a per-Vault section so a later durable
//! fact joins it without a second file or a format migration.
//!
//! This file is deliberately *not* the Vault registry. Losing it costs one
//! extra Git turn per Vault; losing the registry costs the collection. That
//! asymmetry is why the two are separate: the registry is revision-gated and
//! fails closed (an unreadable registry activates no Vaults at all), which is
//! right for configuration and credentials and wrong for bookkeeping a poll
//! can rewrite twice a day.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use serde::{Deserialize, Serialize};

use crate::vault_registry::VaultId;

/// The on-disk shape this build writes and understands.
pub const RUNTIME_STATE_SCHEMA_VERSION: u32 = 1;

/// The file name, resolved beside the Vault registry.
pub const RUNTIME_STATE_FILE_NAME: &str = "vault-runtime.json";

/// How one completed Git turn ended, as persisted. Mirrors
/// `git::ManagedGitOutcome` plus the failure case, which the in-memory
/// outcome type does not carry.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GitTurnOutcome {
    UpToDate,
    Synchronized,
    Failed,
}

/// One Vault's last interval-arming Git turn.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GitTurnRecord {
    pub completed_at: SystemTime,
    pub outcome: GitTurnOutcome,
    /// The failure's error code, present only for [`GitTurnOutcome::Failed`].
    pub code: Option<String>,
}

/// Reader/writer for the durable per-Vault runtime state file.
#[derive(Clone)]
pub struct VaultRuntimeStateStore {
    path: PathBuf,
}

impl VaultRuntimeStateStore {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    /// Resolve the state file beside the Vault registry, so both live in the
    /// same durable state directory a deployment already backs up.
    pub fn beside_registry(registry_path: &Path) -> Self {
        let parent = registry_path.parent().unwrap_or_else(|| Path::new("."));
        Self::new(parent.join(RUNTIME_STATE_FILE_NAME))
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The Vault's last interval-arming Git turn, or `None` when there is no
    /// usable record.
    pub fn last_git_turn(&self, vault_id: VaultId) -> Option<GitTurnRecord> {
        let LoadedState::Usable(stored) = self.load() else {
            return None;
        };
        let turn = stored.vaults.get(&vault_id)?.git.as_ref()?;
        Some(GitTurnRecord {
            completed_at: parse_timestamp(&turn.completed_at)?,
            outcome: turn.outcome,
            code: turn.code.clone(),
        })
    }

    /// Record one completed Git turn.
    pub fn record_git_turn(&self, vault_id: VaultId, record: GitTurnRecord) -> Result<(), String> {
        let mut stored = match self.load() {
            LoadedState::Usable(stored) => stored,
            // A newer Hatchdoor wrote this. Its shape is not ours to guess at,
            // and a downgrade silently flattening it would destroy state the
            // newer build still needs. Refuse; the caller logs and carries on
            // with an in-memory schedule.
            LoadedState::FutureSchema(found) => {
                return Err(format!(
                    "Vault runtime state '{}' uses newer schema {found}, but this Hatchdoor \
                     supports schema {RUNTIME_STATE_SCHEMA_VERSION}; leaving it untouched",
                    self.path.display()
                ));
            }
            // Missing or unreadable: this file is disposable by design, so
            // start a fresh one rather than refusing to schedule forever.
            LoadedState::Unusable => StoredRuntimeState {
                schema_version: u64::from(RUNTIME_STATE_SCHEMA_VERSION),
                vaults: BTreeMap::new(),
            },
        };
        stored.vaults.entry(vault_id).or_default().git = Some(StoredGitTurn {
            completed_at: format_timestamp(record.completed_at),
            outcome: record.outcome,
            code: record.code,
        });
        self.persist(&stored)
    }

    /// Drop a Vault's record entirely — for a Vault that has left the
    /// collection, not one that is merely disabled. A disabled Vault comes
    /// back to the same schedule; a disconnected one is gone, and leaving its
    /// record behind would hand a stale countdown to whatever reconnects
    /// under that Vault ID later.
    ///
    /// A no-op when nothing is stored, so callers can prune unconditionally.
    pub fn forget(&self, vault_id: VaultId) -> Result<(), String> {
        let LoadedState::Usable(mut stored) = self.load() else {
            return Ok(());
        };
        if stored.vaults.remove(&vault_id).is_none() {
            return Ok(());
        }
        self.persist(&stored)
    }

    fn load(&self) -> LoadedState {
        let Ok(contents) = std::fs::read(&self.path) else {
            return LoadedState::Unusable;
        };
        // Read the version before the records: a future file must be
        // recognized as such even when the rest of its shape is unfamiliar.
        let Ok(probe) = serde_json::from_slice::<SchemaProbe>(&contents) else {
            return LoadedState::Unusable;
        };
        if probe.schema_version > u64::from(RUNTIME_STATE_SCHEMA_VERSION) {
            return LoadedState::FutureSchema(probe.schema_version);
        }
        serde_json::from_slice::<StoredRuntimeState>(&contents)
            .map_or(LoadedState::Unusable, LoadedState::Usable)
    }

    fn persist(&self, stored: &StoredRuntimeState) -> Result<(), String> {
        let parent = self.path.parent().unwrap_or_else(|| Path::new("."));
        std::fs::create_dir_all(parent).map_err(|error| {
            format!(
                "could not create Vault runtime state directory '{}': {error}",
                parent.display()
            )
        })?;
        let encoded = serde_json::to_vec_pretty(stored)
            .map_err(|error| format!("could not encode Vault runtime state: {error}"))?;
        std::fs::write(&self.path, encoded)
            .map_err(|error| format!("could not write Vault runtime state: {error}"))
    }
}

/// Seconds precision, matching `model_setup`'s own persisted receipt: a poll
/// clock has no use for sub-second resolution, and the coarser stamp stays
/// readable to whoever opens the file.
fn format_timestamp(at: SystemTime) -> String {
    chrono::DateTime::<chrono::Utc>::from(at).to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

fn parse_timestamp(raw: &str) -> Option<SystemTime> {
    chrono::DateTime::parse_from_rfc3339(raw)
        .ok()
        .map(|parsed| SystemTime::from(parsed.to_utc()))
}

enum LoadedState {
    Usable(StoredRuntimeState),
    FutureSchema(u64),
    Unusable,
}

#[derive(Deserialize)]
struct SchemaProbe {
    schema_version: u64,
}

#[derive(Serialize, Deserialize)]
struct StoredRuntimeState {
    schema_version: u64,
    vaults: BTreeMap<VaultId, StoredVaultRecord>,
}

#[derive(Default, Serialize, Deserialize)]
struct StoredVaultRecord {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    git: Option<StoredGitTurn>,
}

#[derive(Serialize, Deserialize)]
struct StoredGitTurn {
    completed_at: String,
    outcome: GitTurnOutcome,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    code: Option<String>,
}

#[cfg(test)]
mod tests;
