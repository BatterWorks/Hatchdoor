//! Durable, live-applicable server configuration.
//!
//! The process environment is captured once at startup. A durable JSON overlay
//! is then resolved beneath those environment pins and above the defaults. Live
//! consumers call [`RuntimeConfig::snapshot`] once per operation, which gives
//! them an immutable view even when a concurrent settings save succeeds.

use std::collections::BTreeMap;
use std::env;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use arc_swap::ArcSwap;
use serde::{Deserialize, Serialize};

/// The on-disk `settings.json` format version.
pub const SETTINGS_SCHEMA: u32 = 1;

/// Canonical live-applicable keys and their string defaults. Parsers in the
/// consuming configuration modules remain authoritative for validation and
/// typed interpretation of these values.
pub fn live_settings_defaults() -> BTreeMap<String, String> {
    [
        ("HATCHDOOR_ARCHIVE_PREFIX", "90-archive/"),
        ("HATCHDOOR_EXCLUDE", ""),
        ("HATCHDOOR_EMBED_LAYERS", "true"),
        ("HATCHDOOR_MCP_ENABLED", "false"),
        ("HATCHDOOR_MCP_WRITE_ENABLED", "false"),
        ("HATCHDOOR_MAX_ATTACHMENT_BYTES", "10485760"),
        ("HATCHDOOR_MCP_MAX_BASE64_BYTES", "5242880"),
        ("HATCHDOOR_MCP_BEARER_TOKEN", ""),
        (
            "HATCHDOOR_MCP_ALLOWED_ORIGINS",
            "http://127.0.0.1,http://localhost",
        ),
        ("HATCHDOOR_GIT_SYNC_ENABLED", "false"),
        ("HATCHDOOR_GIT_HTTPS_TOKEN", ""),
        ("HATCHDOOR_GIT_REMOTE", "origin"),
        ("HATCHDOOR_GIT_BRANCH", "main"),
        ("HATCHDOOR_GIT_HTTPS_USERNAME", "hatchdoor"),
        ("HATCHDOOR_GIT_DEBOUNCE_SECONDS", "30"),
        ("HATCHDOOR_GIT_AUTHOR_NAME", "Hatchdoor"),
        ("HATCHDOOR_GIT_AUTHOR_EMAIL", "hatchdoor@localhost"),
    ]
    .into_iter()
    .map(|(key, value)| (key.to_string(), value.to_string()))
    .collect()
}

/// Parse one of the truthy string conventions accepted across live-applicable
/// boolean settings (`1`, `true`, `yes`, `on`, case-insensitive, surrounding
/// whitespace ignored). Anything else, including an empty string, is falsy.
pub fn is_truthy(value: &str) -> bool {
    matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "1" | "true" | "yes" | "on"
    )
}

/// Where an effective setting came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SettingSource {
    Environment,
    Stored,
    Default,
}

/// One resolved setting in an immutable runtime snapshot.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResolvedSetting {
    pub value: String,
    pub source: SettingSource,
    /// A non-empty environment value is authoritative for this process.
    pub pinned: bool,
}

/// The configuration bound by one request, job, or other live operation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConfigSnapshot {
    settings: BTreeMap<String, ResolvedSetting>,
}

impl ConfigSnapshot {
    pub fn setting(&self, key: &str) -> Option<&ResolvedSetting> {
        self.settings.get(key)
    }

    /// Look up a required key's resolved value, or a descriptive error naming
    /// the missing key. Every live-applicable key has a default (see
    /// [`live_settings_defaults`]), so this only fails when a caller passes a
    /// key outside that set.
    pub fn required(&self, key: &str) -> Result<&str, String> {
        self.setting(key)
            .map(|setting| setting.value.as_str())
            .ok_or_else(|| format!("runtime configuration is missing {key}"))
    }

    pub fn settings(&self) -> &BTreeMap<String, ResolvedSetting> {
        &self.settings
    }

    /// How many live-applicable settings are pinned by a non-empty environment
    /// variable in this process (and so cannot be changed from the settings
    /// page without editing `.env` and restarting). Surfaced in a startup log
    /// line so an operator can see the count without opening the page.
    pub fn pinned_count(&self) -> usize {
        self.settings
            .values()
            .filter(|setting| setting.pinned)
            .count()
    }

    /// Create a prospective snapshot for all-or-nothing validation before a
    /// settings save. Provenance is deliberately preserved because this view is
    /// never published or persisted.
    pub(crate) fn with_updates(&self, updates: &BTreeMap<String, String>) -> Self {
        let mut settings = self.settings.clone();
        for (key, value) in updates {
            if let Some(setting) = settings.get_mut(key) {
                setting.value = value.clone();
            }
        }
        Self { settings }
    }
}

/// Environment values captured once at startup.
#[derive(Clone, Debug, Default)]
pub struct Environment {
    values: BTreeMap<String, String>,
}

impl Environment {
    pub fn capture<'a>(keys: impl IntoIterator<Item = &'a str>) -> Self {
        Self::from_values(
            keys.into_iter()
                .filter_map(|key| env::var(key).ok().map(|value| (key.to_string(), value))),
        )
    }

    pub fn from_values(values: impl IntoIterator<Item = (String, String)>) -> Self {
        Self {
            values: values
                .into_iter()
                // `KEY=` and `KEY=   ` deliberately behave as unset. This
                // matches the existing parser convention and lets an operator
                // unpin a setting by commenting/removing its value in `.env`.
                .filter(|(_, value)| !value.trim().is_empty())
                .collect(),
        }
    }

    pub fn empty() -> Self {
        Self::default()
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct StoredSettings {
    schema: u32,
    values: BTreeMap<String, String>,
}

struct SettingsStore {
    path: PathBuf,
}

impl SettingsStore {
    fn load(path: impl Into<PathBuf>) -> Result<(Self, BTreeMap<String, String>), String> {
        let store = Self { path: path.into() };
        if !store.path.exists() {
            return Ok((store, BTreeMap::new()));
        }

        let bytes = fs::read(&store.path).map_err(|error| {
            format!(
                "could not read settings store '{}': {error}. Restore its backup or fix its permissions before restarting Hatchdoor.",
                store.path.display()
            )
        })?;
        let parsed: StoredSettings = serde_json::from_slice(&bytes).map_err(|error| {
            format!(
                "settings store '{}' is corrupt: {error}. Move it aside and restart Hatchdoor to begin with defaults, or restore a known-good backup; Hatchdoor will not overwrite it.",
                store.path.display()
            )
        })?;

        if parsed.schema > SETTINGS_SCHEMA {
            return Err(format!(
                "settings store '{}' uses newer schema {} but this Hatchdoor supports schema {}. Upgrade Hatchdoor to read it, or restore a compatible settings.json backup; Hatchdoor will not overwrite it.",
                store.path.display(),
                parsed.schema,
                SETTINGS_SCHEMA
            ));
        }
        if parsed.schema != SETTINGS_SCHEMA {
            return Err(format!(
                "settings store '{}' uses unsupported schema {}. Restore a compatible settings.json backup or remove the file only if you intentionally want to reset stored settings; Hatchdoor will not overwrite it.",
                store.path.display(),
                parsed.schema
            ));
        }

        Ok((store, parsed.values))
    }

    fn persist(&self, values: &BTreeMap<String, String>) -> Result<(), String> {
        let parent = self.path.parent().unwrap_or_else(|| Path::new("."));
        fs::create_dir_all(parent).map_err(|error| {
            format!(
                "could not create settings directory '{}': {error}",
                parent.display()
            )
        })?;

        let encoded = serde_json::to_vec_pretty(&StoredSettings {
            schema: SETTINGS_SCHEMA,
            values: values.clone(),
        })
        .map_err(|error| format!("could not encode settings store: {error}"))?;

        let temporary = temporary_path(&self.path)?;
        let write_result = (|| {
            let mut file = create_private_file(&temporary)?;
            file.write_all(&encoded)
                .map_err(|error| format!("could not write settings store: {error}"))?;
            file.write_all(b"\n")
                .map_err(|error| format!("could not finalize settings store: {error}"))?;
            file.sync_all()
                .map_err(|error| format!("could not sync settings store: {error}"))?;
            fs::rename(&temporary, &self.path).map_err(|error| {
                format!(
                    "could not replace settings store '{}': {error}",
                    self.path.display()
                )
            })?;
            sync_directory(parent)?;
            Ok(())
        })();

        if write_result.is_err() {
            let _ = fs::remove_file(&temporary);
        }
        write_result
    }
}

fn temporary_path(path: &Path) -> Result<PathBuf, String> {
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| format!("settings path '{}' has no valid file name", path.display()))?;
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    for nonce in 0..100_u32 {
        let candidate = parent.join(format!(".{file_name}.{}.{}.tmp", std::process::id(), nonce));
        if !candidate.exists() {
            return Ok(candidate);
        }
    }
    Err(format!(
        "could not create a unique temporary settings store beside '{}'",
        path.display()
    ))
}

#[cfg(unix)]
fn create_private_file(path: &Path) -> Result<std::fs::File, String> {
    use std::os::unix::fs::OpenOptionsExt;

    OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
        .map_err(|error| {
            format!(
                "could not create temporary settings store '{}': {error}",
                path.display()
            )
        })
}

#[cfg(not(unix))]
fn create_private_file(path: &Path) -> Result<std::fs::File, String> {
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|error| {
            format!(
                "could not create temporary settings store '{}': {error}",
                path.display()
            )
        })
}

#[cfg(unix)]
fn sync_directory(path: &Path) -> Result<(), String> {
    let directory = std::fs::File::open(path).map_err(|error| {
        format!(
            "could not open settings directory '{}': {error}",
            path.display()
        )
    })?;
    directory.sync_all().map_err(|error| {
        format!(
            "could not sync settings directory '{}': {error}",
            path.display()
        )
    })
}

#[cfg(not(unix))]
fn sync_directory(_path: &Path) -> Result<(), String> {
    Ok(())
}

/// Lock-free reader / serialized writer for live configuration.
#[derive(Clone)]
pub struct RuntimeConfig {
    environment: Environment,
    defaults: Arc<BTreeMap<String, String>>,
    snapshot: Arc<ArcSwap<ConfigSnapshot>>,
    stored_values: Arc<Mutex<BTreeMap<String, String>>>,
    store: Arc<SettingsStore>,
}

impl RuntimeConfig {
    /// Capture the process environment while building the initial runtime
    /// snapshot. Call this once during startup, never from a request path.
    pub fn load_from_process(
        settings_path: impl Into<PathBuf>,
        defaults: BTreeMap<String, String>,
    ) -> Result<Self, String> {
        let environment = Environment::capture(defaults.keys().map(String::as_str));
        Self::load(settings_path, environment, defaults)
    }

    pub fn load(
        settings_path: impl Into<PathBuf>,
        environment: Environment,
        defaults: BTreeMap<String, String>,
    ) -> Result<Self, String> {
        let (store, stored_values) = SettingsStore::load(settings_path)?;
        let snapshot = resolve(&environment, &stored_values, &defaults);
        Ok(Self {
            environment,
            defaults: Arc::new(defaults),
            snapshot: Arc::new(ArcSwap::from_pointee(snapshot)),
            stored_values: Arc::new(Mutex::new(stored_values)),
            store: Arc::new(store),
        })
    }

    /// Bind the current immutable configuration once at the start of an
    /// operation. Holding this `Arc` never observes a later save.
    pub fn snapshot(&self) -> Arc<ConfigSnapshot> {
        self.snapshot.load_full()
    }

    /// The durable settings file's path, so a caller (e.g. local git
    /// versioning's `.gitignore` setup) can derive ignore entries from the
    /// actual configured location rather than guessing.
    pub fn settings_path(&self) -> &Path {
        &self.store.path
    }

    /// Persist updates before making them live. The writer mutex serializes all
    /// saves and the captured `Environment` means a save never re-reads process
    /// variables at runtime.
    pub fn save(
        &self,
        updates: impl IntoIterator<Item = (String, String)>,
    ) -> Result<Arc<ConfigSnapshot>, String> {
        self.validate_and_save(updates, |_current| Ok(()))
            .map(|((), snapshot)| snapshot)
    }

    /// Remove values that a one-time migration has copied into a durable
    /// domain record. Environment pins and unrelated stored settings remain
    /// untouched. When none of the requested keys are stored, this is a
    /// read-only no-op and does not create `settings.json`.
    pub fn remove_stored<S>(
        &self,
        keys: impl IntoIterator<Item = S>,
    ) -> Result<Arc<ConfigSnapshot>, String>
    where
        S: AsRef<str>,
    {
        let mut stored_values = self
            .stored_values
            .lock()
            .map_err(|_| "configuration save lock was poisoned".to_string())?;
        let mut next_values = stored_values.clone();
        let mut changed = false;
        for key in keys {
            changed |= next_values.remove(key.as_ref()).is_some();
        }
        if !changed {
            return Ok(self.snapshot.load_full());
        }

        self.store.persist(&next_values)?;
        let next_snapshot = Arc::new(resolve(&self.environment, &next_values, &self.defaults));
        self.snapshot.store(next_snapshot.clone());
        *stored_values = next_values;
        Ok(next_snapshot)
    }

    /// Validate against the *current* snapshot and persist in the same
    /// critical section, so two concurrent saves can never both validate
    /// against a snapshot the other is about to invalidate (issue #54: saves
    /// serialize behind one lock end to end, not just at the point of
    /// writing the file).
    ///
    /// `decide` receives the freshly-resolved current snapshot (reflecting
    /// every save that committed before this call acquired the lock) and
    /// returns either a value to hand back alongside the new snapshot, or a
    /// refusal that aborts before anything is written. `updates` is applied
    /// only when `decide` succeeds.
    pub fn validate_and_save<T, E>(
        &self,
        updates: impl IntoIterator<Item = (String, String)>,
        decide: impl FnOnce(&ConfigSnapshot) -> Result<T, E>,
    ) -> Result<(T, Arc<ConfigSnapshot>), E>
    where
        E: From<String>,
    {
        let mut stored_values = self
            .stored_values
            .lock()
            .map_err(|_| E::from("configuration save lock was poisoned".to_string()))?;
        let current = resolve(&self.environment, &stored_values, &self.defaults);
        let value = decide(&current)?;

        let mut next_values = stored_values.clone();
        next_values.extend(updates);
        self.store.persist(&next_values).map_err(E::from)?;
        let next_snapshot = Arc::new(resolve(&self.environment, &next_values, &self.defaults));
        self.snapshot.store(next_snapshot.clone());
        *stored_values = next_values;
        Ok((value, next_snapshot))
    }

    #[cfg(test)]
    pub fn for_tests() -> Self {
        static NEXT_TEST_STORE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let sequence = NEXT_TEST_STORE.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        Self::load(
            std::env::temp_dir().join(format!(
                "hatchdoor-settings-test-{}-{sequence}.json",
                std::process::id()
            )),
            Environment::empty(),
            live_settings_defaults(),
        )
        .expect("test runtime configuration")
    }
}

fn resolve(
    environment: &Environment,
    stored_values: &BTreeMap<String, String>,
    defaults: &BTreeMap<String, String>,
) -> ConfigSnapshot {
    let settings = defaults
        .iter()
        .map(|(key, default)| {
            let resolved = match environment.values.get(key) {
                Some(value) => ResolvedSetting {
                    value: value.clone(),
                    source: SettingSource::Environment,
                    pinned: true,
                },
                None => match stored_values.get(key) {
                    Some(value) => ResolvedSetting {
                        value: value.clone(),
                        source: SettingSource::Stored,
                        pinned: false,
                    },
                    None => ResolvedSetting {
                        value: default.clone(),
                        source: SettingSource::Default,
                        pinned: false,
                    },
                },
            };
            (key.clone(), resolved)
        })
        .collect();
    ConfigSnapshot { settings }
}

/// Default `settings.json` location: next to the disposable cache database.
/// A caller may pass the environment-only `HATCHDOOR_SETTINGS_FILE` value as
/// `override_path` when it captures deployment configuration at startup.
pub fn settings_file_path(cache_db_path: &Path, override_path: Option<&str>) -> PathBuf {
    override_path
        .filter(|path| !path.trim().is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            cache_db_path
                .parent()
                .unwrap_or_else(|| Path::new("."))
                .join("settings.json")
        })
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::fs;

    use tempfile::tempdir;

    use super::{Environment, RuntimeConfig, SettingSource, settings_file_path};

    fn defaults() -> BTreeMap<String, String> {
        [(
            "HATCHDOOR_ARCHIVE_PREFIX".to_string(),
            "90-archive/".to_string(),
        )]
        .into_iter()
        .collect()
    }

    #[test]
    fn stored_values_survive_a_restart() {
        let dir = tempdir().expect("tempdir");
        let path = dir.path().join("settings.json");
        let config = RuntimeConfig::load(&path, Environment::empty(), defaults()).expect("load");

        config
            .save([(
                "HATCHDOOR_ARCHIVE_PREFIX".to_string(),
                "archive/".to_string(),
            )])
            .expect("save");

        let restarted =
            RuntimeConfig::load(&path, Environment::empty(), defaults()).expect("restart");
        let snapshot = restarted.snapshot();
        let setting = snapshot
            .setting("HATCHDOOR_ARCHIVE_PREFIX")
            .expect("setting");
        assert_eq!(setting.value, "archive/");
        assert_eq!(setting.source, SettingSource::Stored);
    }

    #[test]
    fn migration_cleanup_removes_only_selected_stored_values() {
        let dir = tempdir().expect("tempdir");
        let path = dir.path().join("settings.json");
        let defaults = BTreeMap::from([
            ("HATCHDOOR_EXCLUDE".to_string(), "".to_string()),
            (
                "HATCHDOOR_ARCHIVE_PREFIX".to_string(),
                "90-archive/".to_string(),
            ),
        ]);
        let config =
            RuntimeConfig::load(&path, Environment::empty(), defaults.clone()).expect("load");
        config
            .save([
                ("HATCHDOOR_EXCLUDE".to_string(), "private/**".to_string()),
                (
                    "HATCHDOOR_ARCHIVE_PREFIX".to_string(),
                    "archive/".to_string(),
                ),
            ])
            .expect("save fixtures");

        let snapshot = config
            .remove_stored(["HATCHDOOR_EXCLUDE"])
            .expect("remove migrated value");
        assert_eq!(
            snapshot
                .setting("HATCHDOOR_EXCLUDE")
                .expect("exclude")
                .source,
            SettingSource::Default
        );
        assert_eq!(
            snapshot
                .setting("HATCHDOOR_ARCHIVE_PREFIX")
                .expect("archive")
                .source,
            SettingSource::Stored
        );

        let restarted = RuntimeConfig::load(&path, Environment::empty(), defaults)
            .expect("restart after cleanup");
        assert_eq!(
            restarted
                .snapshot()
                .required("HATCHDOOR_ARCHIVE_PREFIX")
                .expect("archive"),
            "archive/"
        );
        assert_eq!(
            restarted
                .snapshot()
                .required("HATCHDOOR_EXCLUDE")
                .expect("exclude"),
            ""
        );
    }

    #[test]
    fn non_empty_environment_value_is_pinned_over_stored_value() {
        let dir = tempdir().expect("tempdir");
        let path = dir.path().join("settings.json");
        let config = RuntimeConfig::load(&path, Environment::empty(), defaults()).expect("load");
        config
            .save([(
                "HATCHDOOR_ARCHIVE_PREFIX".to_string(),
                "stored/".to_string(),
            )])
            .expect("save");

        let environment = Environment::from_values([(
            "HATCHDOOR_ARCHIVE_PREFIX".to_string(),
            "environment/".to_string(),
        )]);
        let restarted = RuntimeConfig::load(&path, environment, defaults()).expect("restart");
        let snapshot = restarted.snapshot();
        let setting = snapshot
            .setting("HATCHDOOR_ARCHIVE_PREFIX")
            .expect("setting");
        assert_eq!(setting.value, "environment/");
        assert_eq!(setting.source, SettingSource::Environment);
        assert!(setting.pinned);
    }

    #[test]
    fn blank_environment_value_does_not_pin_a_setting() {
        let dir = tempdir().expect("tempdir");
        let path = dir.path().join("settings.json");
        let environment =
            Environment::from_values([("HATCHDOOR_ARCHIVE_PREFIX".to_string(), "  ".to_string())]);
        let config = RuntimeConfig::load(&path, environment, defaults()).expect("load");

        let snapshot = config.snapshot();
        let setting = snapshot
            .setting("HATCHDOOR_ARCHIVE_PREFIX")
            .expect("setting");
        assert_eq!(setting.value, "90-archive/");
        assert_eq!(setting.source, SettingSource::Default);
        assert!(!setting.pinned);
    }

    #[cfg(unix)]
    #[test]
    fn created_store_is_private_to_its_owner() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempdir().expect("tempdir");
        let path = dir.path().join("settings.json");
        let config = RuntimeConfig::load(&path, Environment::empty(), defaults()).expect("load");
        config
            .save([(
                "HATCHDOOR_ARCHIVE_PREFIX".to_string(),
                "archive/".to_string(),
            )])
            .expect("save");

        assert_eq!(
            fs::metadata(path).expect("metadata").permissions().mode() & 0o777,
            0o600
        );
    }

    #[test]
    fn corrupt_store_explains_how_to_recover_without_overwriting_it() {
        let dir = tempdir().expect("tempdir");
        let path = dir.path().join("settings.json");
        fs::write(&path, "{ not json").expect("write corrupt store");

        let error = match RuntimeConfig::load(&path, Environment::empty(), defaults()) {
            Ok(_) => panic!("corrupt store accepted"),
            Err(error) => error,
        };
        assert!(error.contains("corrupt"));
        assert!(error.contains("Move it aside"));
        assert_eq!(
            fs::read_to_string(path).expect("store retained"),
            "{ not json"
        );
    }

    #[test]
    fn future_schema_is_rejected_with_upgrade_guidance() {
        let dir = tempdir().expect("tempdir");
        let path = dir.path().join("settings.json");
        fs::write(&path, r#"{"schema":2,"values":{}}"#).expect("write future store");

        let error = match RuntimeConfig::load(&path, Environment::empty(), defaults()) {
            Ok(_) => panic!("future schema accepted"),
            Err(error) => error,
        };
        assert!(error.contains("newer"));
        assert!(error.contains("Upgrade Hatchdoor"));
    }

    #[test]
    fn an_operation_keeps_its_bound_snapshot_across_a_save() {
        let dir = tempdir().expect("tempdir");
        let path = dir.path().join("settings.json");
        let config = RuntimeConfig::load(&path, Environment::empty(), defaults()).expect("load");
        let operation = config.snapshot();

        config
            .save([(
                "HATCHDOOR_ARCHIVE_PREFIX".to_string(),
                "archive/".to_string(),
            )])
            .expect("save");

        assert_eq!(
            operation
                .setting("HATCHDOOR_ARCHIVE_PREFIX")
                .expect("old setting")
                .value,
            "90-archive/"
        );
        assert_eq!(
            config
                .snapshot()
                .setting("HATCHDOOR_ARCHIVE_PREFIX")
                .expect("new setting")
                .value,
            "archive/"
        );
    }

    #[test]
    fn pinned_count_counts_only_environment_sourced_settings() {
        let dir = tempdir().expect("tempdir");
        let path = dir.path().join("settings.json");
        let environment = Environment::from_values([(
            "HATCHDOOR_ARCHIVE_PREFIX".to_string(),
            "env-archive/".to_string(),
        )]);
        let config = RuntimeConfig::load(&path, environment, defaults()).expect("load");

        assert_eq!(config.snapshot().pinned_count(), 1);
    }

    #[test]
    fn settings_file_defaults_beside_cache_unless_overridden() {
        assert_eq!(
            settings_file_path(std::path::Path::new("data/cache/cache.sqlite3"), None),
            std::path::PathBuf::from("data/cache/settings.json")
        );
        assert_eq!(
            settings_file_path(
                std::path::Path::new("data/cache/cache.sqlite3"),
                Some("/state/custom-settings.json")
            ),
            std::path::PathBuf::from("/state/custom-settings.json")
        );
    }
}
