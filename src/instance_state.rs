//! Durable facts about the instance itself, as opposed to any one Vault
//! (`vault_runtime_state`) or its configuration (`vault_registry`, settings).
//!
//! The file holds named sections, each owned by one feature. `versions` holds
//! the version this instance runs, the version it ran before, and the version
//! it was first started on with nothing in place (ADR-40 decision 6). That is
//! what lets the What's new pop-up show what changed since the last version,
//! and stay away from a fresh install (ADR-42). `last_agent` holds the name of
//! the last MCP client that called a tool and when, so Settings can tell a
//! working agent connection from a silent failure (#426). Further sections go
//! through [`InstanceStateStore::section`] and
//! [`InstanceStateStore::write_section`], so nobody copies the write code.
//!
//! Like the Vault runtime state, this is bookkeeping. Losing it costs one
//! What's new pop-up and the last agent record at most, so a missing or unreadable file reads as "no
//! record" and never blocks startup.

use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

/// The on-disk shape this build writes and understands.
pub const INSTANCE_STATE_SCHEMA_VERSION: u32 = 1;

/// The file name, resolved beside the Vault registry.
pub const INSTANCE_STATE_FILE_NAME: &str = "instance.json";

/// The section [`InstanceStateStore::record_start`] keeps.
const VERSIONS_SECTION: &str = "versions";

/// The section [`AgentConnectionLog`] keeps.
const LAST_AGENT_SECTION: &str = "last_agent";

/// The longest client name kept, in characters. Real clients send a few words;
/// anything longer is cut rather than stored whole.
const AGENT_NAME_MAX_CHARS: usize = 100;

/// What a client that sends no usable name is recorded as.
const UNNAMED_AGENT: &str = "Unknown agent";

/// How often a steady stream of calls from the same client is saved. A name
/// change is saved at once.
const AGENT_CONNECTION_WRITE_INTERVAL: Duration = Duration::from_secs(60);

/// The version an install that has a registry or stored settings but no
/// record is taken to be upgrading from. Instance state arrived in 2.8.0, and
/// installs on 2.4.x or earlier cannot upgrade to it directly (ADR-40), so an
/// install with no record ran a 2.7.x release.
pub const UNRECORDED_UPGRADE_FROM: &str = "2.7.0";

/// Which versions this instance has run.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct VersionRecord {
    /// The version running now, as a base version such as `2.8.0`.
    pub current: String,
    /// The version that ran before `current`, or `None` when this instance
    /// has only ever run `current`.
    pub previous: Option<String>,
    /// The version the instance was first started on with no registry and no
    /// stored settings, or `None` for an install that existed before it
    /// started keeping this record.
    pub fresh_install: Option<String>,
}

impl VersionRecord {
    /// What a start on `running` makes of `existing`, the record left by the
    /// last start, or of no record. `existing_install` says whether a
    /// registry or stored settings were already in place, and matters only
    /// when there is no record.
    ///
    /// `previous` moves only when the base version changes, so restarting on
    /// the same version, or on another build of it, changes nothing.
    pub fn after_start(
        existing: Option<&VersionRecord>,
        running: &str,
        existing_install: bool,
    ) -> VersionRecord {
        let running = base_version(running);
        match existing {
            Some(record) if record.current == running => record.clone(),
            Some(record) => VersionRecord {
                current: running.to_string(),
                previous: Some(record.current.clone()),
                fresh_install: record.fresh_install.clone(),
            },
            // A build of the baseline itself, such as a nightly cut before the
            // version bump, has nothing to compare with yet.
            None if existing_install => VersionRecord {
                current: running.to_string(),
                previous: (running != UNRECORDED_UPGRADE_FROM)
                    .then(|| UNRECORDED_UPGRADE_FROM.to_string()),
                fresh_install: None,
            },
            None => VersionRecord {
                current: running.to_string(),
                previous: None,
                fresh_install: Some(running.to_string()),
            },
        }
    }
}

impl Default for VersionRecord {
    /// The running version with no history: what an instance that has not
    /// recorded a start knows about itself.
    fn default() -> Self {
        Self {
            current: base_version(&crate::config::version_string()).to_string(),
            previous: None,
            fresh_install: None,
        }
    }
}

/// The release part of a version string. A development build reports
/// `2.8.0 (dev abc123)` (see [`crate::config::version_string`]), and compares
/// as `2.8.0`.
pub fn base_version(version: &str) -> &str {
    version
        .split_once(" (dev")
        .map_or(version, |(base, _)| base)
        .trim()
}

/// The last MCP client that called a tool, and when. Only the name and the
/// time: nothing about the request itself is kept.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentConnection {
    /// The client's display name, cleaned by [`clean_agent_name`].
    pub name: String,
    /// RFC 3339, seconds precision, UTC.
    pub connected_at: String,
}

/// A client name as it is safe to keep and show: control and invisible
/// formatting characters removed (a bidi override could make the name read
/// as something else), surrounding whitespace trimmed, and at most
/// [`AGENT_NAME_MAX_CHARS`] characters. A name with nothing left becomes
/// [`UNNAMED_AGENT`].
fn clean_agent_name(raw: &str) -> String {
    let cleaned: String = raw
        .chars()
        .filter(|&character| !character.is_control() && !is_invisible_format(character))
        .collect();
    let cleaned: String = cleaned.trim().chars().take(AGENT_NAME_MAX_CHARS).collect();
    let cleaned = cleaned.trim_end();
    if cleaned.is_empty() {
        UNNAMED_AGENT.to_string()
    } else {
        cleaned.to_string()
    }
}

/// Zero-width, bidi, line-separator and tag characters, which render as
/// nothing, break the line, or reorder the text around them.
fn is_invisible_format(character: char) -> bool {
    matches!(
        character,
        '\u{00AD}'
            | '\u{061C}'
            | '\u{180E}'
            | '\u{200B}'..='\u{200F}'
            | '\u{202A}'..='\u{202E}'
            | '\u{2028}'
            | '\u{2029}'
            | '\u{2060}'..='\u{206F}'
            | '\u{FEFF}'
            | '\u{E0000}'..='\u{E007F}'
    )
}

/// Remembers the last MCP client in memory and saves it to the instance state
/// file at most once per [`AGENT_CONNECTION_WRITE_INTERVAL`], or when the name
/// changes, so it survives a restart without a write per call.
///
/// [`Default`] keeps the record in memory only.
#[derive(Default)]
pub struct AgentConnectionLog {
    store: Option<InstanceStateStore>,
    state: Mutex<AgentLogState>,
}

#[derive(Default)]
struct AgentLogState {
    latest: Option<AgentConnection>,
    /// The name and time of the last save decided on, whether or not the
    /// write then succeeded: a failing disk is retried a minute later, not on
    /// every call.
    saved: Option<SavedAgent>,
}

/// The name and call time last handed to [`AgentConnectionLog::save`].
struct SavedAgent {
    name: String,
    at: SystemTime,
}

impl AgentConnectionLog {
    /// A log backed by `store`, starting from the record it already holds.
    pub fn load(store: InstanceStateStore) -> Self {
        let latest = store.section::<AgentConnection>(LAST_AGENT_SECTION);
        Self {
            store: Some(store),
            state: Mutex::new(AgentLogState {
                latest,
                saved: None,
            }),
        }
    }

    /// The last client recorded, or `None` when no agent has connected yet.
    pub fn latest(&self) -> Option<AgentConnection> {
        self.lock().latest.clone()
    }

    /// Note a call from the client named `raw_name` at `at`. Returns `true`
    /// when the record is due to be saved; the caller then calls
    /// [`Self::save`], off the request path if it likes.
    pub fn observe(&self, raw_name: &str, at: SystemTime) -> bool {
        let name = clean_agent_name(raw_name);
        let mut state = self.lock();
        state.latest = Some(AgentConnection {
            name: name.clone(),
            connected_at: chrono::DateTime::<chrono::Utc>::from(at)
                .to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
        });
        let due = self.store.is_some()
            && state.saved.as_ref().is_none_or(|saved| {
                saved.name != name
                    || at
                        .duration_since(saved.at)
                        .is_ok_and(|elapsed| elapsed >= AGENT_CONNECTION_WRITE_INTERVAL)
            });
        if due {
            state.saved = Some(SavedAgent { name, at });
        }
        due
    }

    /// Write the current record to the instance state file. Never fails: a
    /// record that cannot be written is logged and kept in memory.
    pub fn save(&self) {
        let (Some(store), Some(latest)) = (&self.store, self.latest()) else {
            return;
        };
        if let Err(message) = store.write_section(LAST_AGENT_SECTION, &latest) {
            tracing::warn!(
                path = %store.path().display(),
                "{message}; keeping the last agent connection in memory only"
            );
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, AgentLogState> {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

/// Reader/writer for the instance state file.
///
/// Clones share one write lock, so two features updating their own sections
/// at once cannot write back what the other read before its change.
#[derive(Clone)]
pub struct InstanceStateStore {
    path: PathBuf,
    writes: Arc<Mutex<()>>,
}

impl InstanceStateStore {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            writes: Arc::new(Mutex::new(())),
        }
    }

    /// Resolve the file beside the Vault registry, in the same durable state
    /// directory a deployment already backs up.
    pub fn beside_registry(registry_path: &Path) -> Self {
        let parent = registry_path.parent().unwrap_or_else(|| Path::new("."));
        Self::new(parent.join(INSTANCE_STATE_FILE_NAME))
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Record a start on `running` and return the resulting record; see
    /// [`VersionRecord::after_start`]. Runs once at startup, before anything
    /// else writes a registry, so `existing_install` still tells a fresh
    /// install from an existing one.
    ///
    /// Never fails: a record that cannot be written is logged, and the
    /// record this start computed is still returned. A file written by a
    /// newer Hatchdoor is treated as no record and left untouched.
    pub fn record_start(&self, running: &str, existing_install: bool) -> VersionRecord {
        // `section` reads a newer schema as no section, and `write_section`
        // refuses to touch it, so that case needs nothing of its own here.
        let existing = self.section::<VersionRecord>(VERSIONS_SECTION);
        let record = VersionRecord::after_start(existing.as_ref(), running, existing_install);
        if existing.as_ref() != Some(&record)
            && let Err(message) = self.write_section(VERSIONS_SECTION, &record)
        {
            tracing::warn!(
                path = %self.path.display(),
                "{message}; carrying on without a version record"
            );
        }
        record
    }

    /// The section stored under `name`, or `None` when there is no usable
    /// file, no such section, or a section of another shape.
    pub fn section<T: DeserializeOwned>(&self, name: &str) -> Option<T> {
        let LoadedState::Usable(stored) = self.load() else {
            return None;
        };
        serde_json::from_value(stored.sections.get(name)?.clone()).ok()
    }

    /// Replace the section stored under `name`, keeping every other section.
    /// Refuses, writing nothing, when the file belongs to a newer Hatchdoor.
    pub fn write_section<T: Serialize>(&self, name: &str, value: &T) -> Result<(), String> {
        let _write = self.lock_writes();
        let mut stored = match self.load() {
            LoadedState::Usable(stored) => stored,
            LoadedState::FutureSchema(found) => {
                return Err(format!(
                    "Instance state '{}' uses newer schema {found}, but this Hatchdoor \
                     supports schema {INSTANCE_STATE_SCHEMA_VERSION}; leaving it untouched",
                    self.path.display()
                ));
            }
            LoadedState::Unusable => StoredInstanceState::empty(),
        };
        let value = serde_json::to_value(value)
            .map_err(|error| format!("could not encode instance state: {error}"))?;
        stored.sections.insert(name.to_string(), value);
        self.persist(&stored)
    }

    /// Drop the section stored under `name`, keeping every other section.
    /// Writes nothing when there is no such section, and refuses when the
    /// file belongs to a newer Hatchdoor.
    pub fn remove_section(&self, name: &str) -> Result<(), String> {
        let _write = self.lock_writes();
        let mut stored = match self.load() {
            LoadedState::Usable(stored) => stored,
            LoadedState::FutureSchema(found) => {
                return Err(format!(
                    "Instance state '{}' uses newer schema {found}, but this Hatchdoor \
                     supports schema {INSTANCE_STATE_SCHEMA_VERSION}; leaving it untouched",
                    self.path.display()
                ));
            }
            LoadedState::Unusable => return Ok(()),
        };
        if stored.sections.remove(name).is_none() {
            return Ok(());
        }
        self.persist(&stored)
    }

    /// Held across a whole read-modify-write. A panic while holding it cannot
    /// leave the file half-written (see [`Self::persist`]), so a poisoned lock
    /// is still safe to take.
    fn lock_writes(&self) -> std::sync::MutexGuard<'_, ()> {
        self.writes
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn load(&self) -> LoadedState {
        let Ok(contents) = std::fs::read(&self.path) else {
            return LoadedState::Unusable;
        };
        // Read the version before the sections: a future file must be
        // recognized as such even when the rest of its shape is unfamiliar.
        let Ok(probe) = serde_json::from_slice::<SchemaProbe>(&contents) else {
            return LoadedState::Unusable;
        };
        if probe.schema_version > u64::from(INSTANCE_STATE_SCHEMA_VERSION) {
            return LoadedState::FutureSchema(probe.schema_version);
        }
        serde_json::from_slice::<StoredInstanceState>(&contents)
            .map_or(LoadedState::Unusable, LoadedState::Usable)
    }

    fn persist(&self, stored: &StoredInstanceState) -> Result<(), String> {
        let parent = self.path.parent().unwrap_or_else(|| Path::new("."));
        std::fs::create_dir_all(parent).map_err(|error| {
            format!(
                "could not create instance state directory '{}': {error}",
                parent.display()
            )
        })?;
        let encoded = serde_json::to_vec_pretty(stored)
            .map_err(|error| format!("could not encode instance state: {error}"))?;
        // Written beside the file and renamed over it, so a reader, or the
        // next start after a crash mid-write, sees the old file or the new
        // one and never a truncated one.
        static TEMPORARY_SEQUENCE: AtomicU64 = AtomicU64::new(0);
        let file_name = self.path.file_name().map_or_else(
            || INSTANCE_STATE_FILE_NAME.into(),
            |name| name.to_os_string(),
        );
        let mut temporary_name = std::ffi::OsString::from(".");
        temporary_name.push(file_name);
        temporary_name.push(format!(
            ".{}.{}.tmp",
            std::process::id(),
            TEMPORARY_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        let temporary = parent.join(temporary_name);
        let result = (|| {
            let mut file = std::fs::File::create(&temporary)
                .map_err(|error| format!("could not write instance state: {error}"))?;
            file.write_all(&encoded)
                .and_then(|()| file.sync_all())
                .map_err(|error| format!("could not write instance state: {error}"))?;
            std::fs::rename(&temporary, &self.path)
                .map_err(|error| format!("could not replace instance state: {error}"))
        })();
        if result.is_err() {
            let _ = std::fs::remove_file(&temporary);
        }
        result
    }
}

enum LoadedState {
    Usable(StoredInstanceState),
    FutureSchema(u64),
    Unusable,
}

#[derive(Deserialize)]
struct SchemaProbe {
    schema_version: u64,
}

#[derive(Serialize, Deserialize)]
struct StoredInstanceState {
    schema_version: u64,
    #[serde(flatten)]
    sections: BTreeMap<String, serde_json::Value>,
}

impl StoredInstanceState {
    fn empty() -> Self {
        Self {
            schema_version: u64::from(INSTANCE_STATE_SCHEMA_VERSION),
            sections: BTreeMap::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn store_in(directory: &Path) -> InstanceStateStore {
        InstanceStateStore::new(directory.join("state/instance.json"))
    }

    #[test]
    fn a_fresh_install_records_its_version_and_stays_fresh_across_restarts() {
        let directory = tempdir().expect("temporary state directory");
        let first = store_in(directory.path()).record_start("2.8.0", false);
        assert_eq!(
            first,
            VersionRecord {
                current: "2.8.0".into(),
                previous: None,
                fresh_install: Some("2.8.0".into()),
            }
        );
        // A restart sees the record the first start wrote, not an install
        // that now has state of its own.
        let restarted = store_in(directory.path()).record_start("2.8.0", true);
        assert_eq!(restarted, first);
    }

    #[test]
    fn an_existing_install_with_no_record_is_an_upgrade_from_2_7() {
        let directory = tempdir().expect("temporary state directory");
        let record = store_in(directory.path()).record_start("2.8.0", true);
        assert_eq!(
            record,
            VersionRecord {
                current: "2.8.0".into(),
                previous: Some("2.7.0".into()),
                fresh_install: None,
            }
        );
    }

    #[test]
    fn an_existing_install_on_the_baseline_has_no_previous_version_yet() {
        let directory = tempdir().expect("temporary state directory");
        let record = store_in(directory.path()).record_start("2.7.0 (dev abc123)", true);
        assert_eq!(record.current, "2.7.0");
        assert_eq!(record.previous, None);
        assert_eq!(record.fresh_install, None);
    }

    #[test]
    fn previous_moves_only_when_the_version_changes() {
        let directory = tempdir().expect("temporary state directory");
        store_in(directory.path()).record_start("2.7.0", true);
        let upgraded = store_in(directory.path()).record_start("2.8.0", true);
        assert_eq!(upgraded.previous.as_deref(), Some("2.7.0"));
        for _ in 0..2 {
            let restarted = store_in(directory.path()).record_start("2.8.0", true);
            assert_eq!(restarted, upgraded);
        }
        let next = store_in(directory.path()).record_start("2.9.0", true);
        assert_eq!(next.current, "2.9.0");
        assert_eq!(next.previous.as_deref(), Some("2.8.0"));
    }

    #[test]
    fn a_fresh_install_keeps_its_first_version_through_upgrades() {
        let directory = tempdir().expect("temporary state directory");
        store_in(directory.path()).record_start("2.8.0", false);
        let upgraded = store_in(directory.path()).record_start("2.9.0", true);
        assert_eq!(upgraded.previous.as_deref(), Some("2.8.0"));
        assert_eq!(upgraded.fresh_install.as_deref(), Some("2.8.0"));
    }

    #[test]
    fn a_development_build_compares_as_its_release() {
        assert_eq!(base_version("2.8.0 (dev abc123)"), "2.8.0");
        assert_eq!(base_version("2.8.0"), "2.8.0");
        let directory = tempdir().expect("temporary state directory");
        store_in(directory.path()).record_start("2.8.0", true);
        let dev = store_in(directory.path()).record_start("2.8.0 (dev abc123)", true);
        assert_eq!(dev.current, "2.8.0");
        assert_eq!(dev.previous.as_deref(), Some("2.7.0"));
    }

    #[test]
    fn a_newer_schema_is_treated_as_no_record_and_left_untouched() {
        let directory = tempdir().expect("temporary state directory");
        let store = store_in(directory.path());
        std::fs::create_dir_all(store.path().parent().unwrap()).unwrap();
        let future = r#"{"schema_version": 99, "versions": {"current": "9.0.0"}}"#;
        std::fs::write(store.path(), future).unwrap();

        let record = store.record_start("2.8.0", true);
        assert_eq!(record.previous.as_deref(), Some("2.7.0"));
        assert_eq!(std::fs::read_to_string(store.path()).unwrap(), future);
        assert!(store.write_section("other", &1).is_err());
        assert_eq!(std::fs::read_to_string(store.path()).unwrap(), future);
        assert_eq!(store.section::<u32>("other"), None);
    }

    #[test]
    fn an_unreadable_file_is_no_record_and_is_replaced() {
        let directory = tempdir().expect("temporary state directory");
        let store = store_in(directory.path());
        std::fs::create_dir_all(store.path().parent().unwrap()).unwrap();
        std::fs::write(store.path(), "not json").unwrap();

        let record = store.record_start("2.8.0", false);
        assert_eq!(record.fresh_install.as_deref(), Some("2.8.0"));
        assert_eq!(store.section::<VersionRecord>("versions"), Some(record));
    }

    #[test]
    fn a_state_directory_that_cannot_be_written_does_not_fail_the_start() {
        let directory = tempdir().expect("temporary state directory");
        // A file where the state directory should be.
        std::fs::write(directory.path().join("state"), "").unwrap();
        let record = store_in(directory.path()).record_start("2.8.0", false);
        assert_eq!(record.fresh_install.as_deref(), Some("2.8.0"));
    }

    fn observe_and_save(log: &AgentConnectionLog, name: &str, at: SystemTime) -> bool {
        let due = log.observe(name, at);
        if due {
            log.save();
        }
        due
    }

    #[test]
    fn the_last_agent_survives_a_restart() {
        let directory = tempdir().expect("temporary state directory");
        let log = AgentConnectionLog::load(store_in(directory.path()));
        assert_eq!(log.latest(), None);
        let at = SystemTime::UNIX_EPOCH + Duration::from_secs(1_790_000_000);
        observe_and_save(&log, "Claude Code", at);

        let restarted = AgentConnectionLog::load(store_in(directory.path()));
        assert_eq!(
            restarted.latest(),
            Some(AgentConnection {
                name: "Claude Code".into(),
                connected_at: "2026-09-21T14:13:20Z".into(),
            })
        );
    }

    #[test]
    fn a_burst_of_calls_writes_the_file_at_most_once_a_minute() {
        let directory = tempdir().expect("temporary state directory");
        let log = AgentConnectionLog::load(store_in(directory.path()));
        let start = SystemTime::UNIX_EPOCH + Duration::from_secs(1_790_000_000);
        let writes = (0..120)
            .filter(|second| {
                observe_and_save(&log, "codex", start + Duration::from_millis(second * 500))
            })
            .count();
        assert_eq!(writes, 1, "one write for a minute of calls");
        // Memory has the newest call; the file still has the first.
        assert_eq!(
            log.latest().map(|record| record.connected_at),
            Some("2026-09-21T14:14:19Z".into())
        );
        let on_disk = AgentConnectionLog::load(store_in(directory.path())).latest();
        assert_eq!(
            on_disk.map(|record| record.connected_at),
            Some("2026-09-21T14:13:20Z".into())
        );

        assert!(observe_and_save(
            &log,
            "codex",
            start + Duration::from_secs(60)
        ));
        let on_disk = AgentConnectionLog::load(store_in(directory.path())).latest();
        assert_eq!(
            on_disk.map(|record| record.connected_at),
            Some("2026-09-21T14:14:20Z".into())
        );
    }

    #[test]
    fn a_new_client_name_is_written_at_once() {
        let directory = tempdir().expect("temporary state directory");
        let log = AgentConnectionLog::load(store_in(directory.path()));
        let start = SystemTime::UNIX_EPOCH + Duration::from_secs(1_790_000_000);
        assert!(observe_and_save(&log, "codex", start));
        assert!(!observe_and_save(
            &log,
            "codex",
            start + Duration::from_secs(1)
        ));
        assert!(observe_and_save(
            &log,
            "Claude Code",
            start + Duration::from_secs(2)
        ));
        let on_disk = AgentConnectionLog::load(store_in(directory.path())).latest();
        assert_eq!(
            on_disk.map(|record| record.name),
            Some("Claude Code".into())
        );
    }

    #[test]
    fn the_last_agent_keeps_the_version_record_intact() {
        let directory = tempdir().expect("temporary state directory");
        let store = store_in(directory.path());
        let versions = store.record_start("2.8.0", false);
        let log = AgentConnectionLog::load(store.clone());
        observe_and_save(&log, "codex", SystemTime::now());
        assert_eq!(store.section::<VersionRecord>("versions"), Some(versions));
    }

    #[test]
    fn an_in_memory_log_never_asks_for_a_write() {
        let log = AgentConnectionLog::default();
        assert!(!log.observe("codex", SystemTime::now()));
        assert_eq!(log.latest().map(|record| record.name), Some("codex".into()));
    }

    #[test]
    fn a_hostile_client_name_is_stored_trimmed_and_clean() {
        assert_eq!(clean_agent_name("  Claude Code \n"), "Claude Code");
        assert_eq!(
            clean_agent_name("evil\u{1b}[31m\u{0}name\u{7}"),
            "evil[31mname"
        );
        assert_eq!(
            clean_agent_name("good\u{202E}txt.exe\u{200B}"),
            "goodtxt.exe"
        );
        assert_eq!(clean_agent_name("\u{0}\t \u{FEFF}"), UNNAMED_AGENT);
        assert_eq!(clean_agent_name(""), UNNAMED_AGENT);
        let long = "é".repeat(10_000);
        assert_eq!(
            clean_agent_name(&long).chars().count(),
            AGENT_NAME_MAX_CHARS
        );

        let directory = tempdir().expect("temporary state directory");
        let log = AgentConnectionLog::load(store_in(directory.path()));
        observe_and_save(
            &log,
            &format!("x\u{1b}{}", "y".repeat(5_000)),
            SystemTime::now(),
        );
        let stored = AgentConnectionLog::load(store_in(directory.path()))
            .latest()
            .expect("record saved");
        assert_eq!(stored.name.chars().count(), AGENT_NAME_MAX_CHARS);
        assert!(stored.name.chars().all(|character| !character.is_control()));
    }

    #[test]
    fn sections_written_by_other_features_survive_a_start() {
        let directory = tempdir().expect("temporary state directory");
        let store = store_in(directory.path());
        store.record_start("2.8.0", false);
        store
            .write_section("update_check", &serde_json::json!({"latest": "2.9.0"}))
            .expect("write a section");
        store.record_start("2.9.0", true);
        assert_eq!(
            store.section::<serde_json::Value>("update_check"),
            Some(serde_json::json!({"latest": "2.9.0"}))
        );
        assert_eq!(
            store
                .section::<VersionRecord>("versions")
                .and_then(|record| record.previous),
            Some("2.8.0".into())
        );
    }

    #[test]
    fn a_removed_section_is_gone_and_the_others_stay() {
        let directory = tempdir().expect("temporary state directory");
        let store = store_in(directory.path());
        store.record_start("2.8.0", false);
        store
            .write_section("feature", &serde_json::json!({ "kept": false }))
            .expect("write section");

        store.remove_section("feature").expect("remove section");

        assert_eq!(store.section::<serde_json::Value>("feature"), None);
        assert!(store.section::<VersionRecord>(VERSIONS_SECTION).is_some());
    }

    #[test]
    fn removing_a_section_that_is_not_there_writes_nothing() {
        let directory = tempdir().expect("temporary state directory");
        let store = store_in(directory.path());

        store.remove_section("feature").expect("nothing to remove");

        assert!(!store.path().exists());
    }
}
