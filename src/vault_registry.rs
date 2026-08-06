//! Authoritative, revisioned persistence for the collection of Vault identities.
//!
//! Definition fields and lifecycle operations intentionally arrive in issue #86.
//! This boundary owns only durable identity, revision, atomicity, permissions,
//! and recovery behavior.

use std::collections::BTreeMap;
use std::fmt;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Component, Path, PathBuf};
use std::str::FromStr;
use std::sync::{Arc, Mutex, OnceLock};

use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// The on-disk `vaults.json` format version.
pub const REGISTRY_SCHEMA_VERSION: u32 = 1;
/// The authoritative Vault collection registry in persistent instance state.
pub const DEFAULT_VAULT_REGISTRY_PATH: &str = "/data/state/vaults.json";

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct VaultId([u8; 16]);

impl VaultId {
    pub fn generate() -> Result<Self, VaultIdError> {
        let mut bytes = [0_u8; 16];
        getrandom::fill(&mut bytes).map_err(|error| VaultIdError::Randomness(error.to_string()))?;
        bytes[6] = (bytes[6] & 0x0f) | 0x40;
        bytes[8] = (bytes[8] & 0x3f) | 0x80;
        Ok(Self(bytes))
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum VaultIdError {
    InvalidFormat,
    Randomness(String),
}

impl fmt::Display for VaultIdError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidFormat => formatter.write_str("Vault ID must be a canonical UUID v4"),
            Self::Randomness(error) => write!(formatter, "could not generate Vault ID: {error}"),
        }
    }
}

impl std::error::Error for VaultIdError {}

impl fmt::Display for VaultId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let bytes = self.0;
        write!(
            formatter,
            "{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
            bytes[0],
            bytes[1],
            bytes[2],
            bytes[3],
            bytes[4],
            bytes[5],
            bytes[6],
            bytes[7],
            bytes[8],
            bytes[9],
            bytes[10],
            bytes[11],
            bytes[12],
            bytes[13],
            bytes[14],
            bytes[15]
        )
    }
}

impl FromStr for VaultId {
    type Err = VaultIdError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        if value.len() != 36
            || !value.is_ascii()
            || [8, 13, 18, 23]
                .into_iter()
                .any(|index| value.as_bytes()[index] != b'-')
        {
            return Err(VaultIdError::InvalidFormat);
        }

        let mut bytes = [0_u8; 16];
        let mut output = 0;
        let encoded = value.as_bytes();
        let mut input = 0;
        while input < encoded.len() {
            if matches!(input, 8 | 13 | 18 | 23) {
                input += 1;
                continue;
            }
            let high = decode_hex(encoded[input]).ok_or(VaultIdError::InvalidFormat)?;
            let low = decode_hex(encoded[input + 1]).ok_or(VaultIdError::InvalidFormat)?;
            bytes[output] = (high << 4) | low;
            input += 2;
            output += 1;
        }

        if bytes[6] >> 4 != 4 || bytes[8] >> 6 != 2 {
            return Err(VaultIdError::InvalidFormat);
        }
        Ok(Self(bytes))
    }
}

impl Serialize for VaultId {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for VaultId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::from_str(&value).map_err(D::Error::custom)
    }
}

fn decode_hex(value: u8) -> Option<u8> {
    match value {
        b'0'..=b'9' => Some(value - b'0'),
        b'a'..=b'f' => Some(value - b'a' + 10),
        _ => None,
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
#[serde(deny_unknown_fields)]
/// The persisted slot for one Vault ID. Issue #86 adds the accepted definition
/// fields without changing the identity or registry persistence contract.
pub struct VaultRecord {}

impl VaultRecord {
    #[cfg(test)]
    fn empty() -> Self {
        Self {}
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VaultRegistrySnapshot {
    schema_version: u32,
    revision: u64,
    vaults: BTreeMap<VaultId, VaultRecord>,
}

impl VaultRegistrySnapshot {
    fn empty() -> Self {
        Self {
            schema_version: REGISTRY_SCHEMA_VERSION,
            revision: 0,
            vaults: BTreeMap::new(),
        }
    }

    pub fn schema_version(&self) -> u32 {
        self.schema_version
    }

    pub fn revision(&self) -> u64 {
        self.revision
    }

    pub fn vault_ids(&self) -> impl ExactSizeIterator<Item = VaultId> + '_ {
        self.vaults.keys().copied()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VaultRegistryRecoveryKind {
    Corrupt,
    UnsupportedSchema { found: u64, supported: u32 },
    FutureSchema { found: u64, supported: u32 },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VaultRegistryRecovery {
    kind: VaultRegistryRecoveryKind,
    message: String,
}

impl VaultRegistryRecovery {
    pub fn kind(&self) -> VaultRegistryRecoveryKind {
        self.kind
    }

    pub fn message(&self) -> &str {
        &self.message
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum VaultRegistryState {
    Ready(VaultRegistrySnapshot),
    Recovery(VaultRegistryRecovery),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum VaultRegistryError {
    RevisionConflict { expected: u64, actual: u64 },
    RecoveryRequired,
    RevisionExhausted,
    LockPoisoned,
    Storage(String),
}

impl fmt::Display for VaultRegistryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::RevisionConflict { expected, actual } => write!(
                formatter,
                "Vault registry revision conflict: expected {expected}, current {actual}"
            ),
            Self::RecoveryRequired => {
                formatter.write_str("Vault registry requires recovery and will not be overwritten")
            }
            Self::RevisionExhausted => formatter.write_str("Vault registry revision is exhausted"),
            Self::LockPoisoned => formatter.write_str("Vault registry write lock was poisoned"),
            Self::Storage(message) => formatter.write_str(message),
        }
    }
}

impl std::error::Error for VaultRegistryError {}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredVaultRegistry {
    schema_version: u32,
    revision: u64,
    vaults: BTreeMap<VaultId, VaultRecord>,
}

#[derive(Clone)]
pub struct VaultRegistryStore {
    path: PathBuf,
    write_lock: Arc<Mutex<()>>,
}

impl VaultRegistryStore {
    pub fn at_default_path() -> Self {
        Self::new(DEFAULT_VAULT_REGISTRY_PATH)
    }

    pub fn new(path: impl Into<PathBuf>) -> Self {
        let path = path.into();
        Self {
            write_lock: write_lock_for(&path),
            path,
        }
    }

    pub fn load(&self) -> Result<VaultRegistryState, VaultRegistryError> {
        self.load_unlocked()
    }

    fn load_unlocked(&self) -> Result<VaultRegistryState, VaultRegistryError> {
        if !self.path.exists() {
            return Ok(VaultRegistryState::Ready(VaultRegistrySnapshot::empty()));
        }

        let encoded = fs::read(&self.path).map_err(|error| {
            VaultRegistryError::Storage(format!(
                "could not read Vault registry '{}': {error}",
                self.path.display()
            ))
        })?;
        let value: serde_json::Value = match serde_json::from_slice(&encoded) {
            Ok(value) => value,
            Err(error) => {
                return Ok(VaultRegistryState::Recovery(
                    self.corrupt(error.to_string()),
                ));
            }
        };
        let Some(schema_version) = value.get("schema_version").and_then(|value| value.as_u64())
        else {
            return Ok(VaultRegistryState::Recovery(
                self.corrupt("missing unsigned schema_version"),
            ));
        };
        if schema_version > u64::from(REGISTRY_SCHEMA_VERSION) {
            return Ok(VaultRegistryState::Recovery(VaultRegistryRecovery {
                kind: VaultRegistryRecoveryKind::FutureSchema {
                    found: schema_version,
                    supported: REGISTRY_SCHEMA_VERSION,
                },
                message: format!(
                    "Vault registry '{}' uses newer schema {schema_version}, but this Hatchdoor supports schema {}. Upgrade Hatchdoor or restore a compatible backup; Hatchdoor will not overwrite it.",
                    self.path.display(),
                    REGISTRY_SCHEMA_VERSION
                ),
            }));
        }
        if schema_version < u64::from(REGISTRY_SCHEMA_VERSION) {
            return Ok(VaultRegistryState::Recovery(VaultRegistryRecovery {
                kind: VaultRegistryRecoveryKind::UnsupportedSchema {
                    found: schema_version,
                    supported: REGISTRY_SCHEMA_VERSION,
                },
                message: format!(
                    "Vault registry '{}' uses unsupported schema {schema_version}, but this Hatchdoor supports schema {}. Restore a compatible backup; Hatchdoor will not overwrite it.",
                    self.path.display(),
                    REGISTRY_SCHEMA_VERSION
                ),
            }));
        }
        let stored: StoredVaultRegistry = match serde_json::from_value(value) {
            Ok(stored) => stored,
            Err(error) => {
                return Ok(VaultRegistryState::Recovery(
                    self.corrupt(error.to_string()),
                ));
            }
        };
        Ok(VaultRegistryState::Ready(VaultRegistrySnapshot {
            schema_version: stored.schema_version,
            revision: stored.revision,
            vaults: stored.vaults,
        }))
    }

    pub fn commit(
        &self,
        expected_revision: u64,
        vaults: impl IntoIterator<Item = (VaultId, VaultRecord)>,
    ) -> Result<VaultRegistrySnapshot, VaultRegistryError> {
        let _guard = self
            .write_lock
            .lock()
            .map_err(|_| VaultRegistryError::LockPoisoned)?;
        let VaultRegistryState::Ready(current) = self.load_unlocked()? else {
            return Err(VaultRegistryError::RecoveryRequired);
        };
        if current.revision != expected_revision {
            return Err(VaultRegistryError::RevisionConflict {
                expected: expected_revision,
                actual: current.revision,
            });
        }
        let revision = current
            .revision
            .checked_add(1)
            .ok_or(VaultRegistryError::RevisionExhausted)?;
        let next = VaultRegistrySnapshot {
            schema_version: REGISTRY_SCHEMA_VERSION,
            revision,
            vaults: vaults.into_iter().collect(),
        };
        self.persist(&next)?;
        Ok(next)
    }

    fn persist(&self, snapshot: &VaultRegistrySnapshot) -> Result<(), VaultRegistryError> {
        let parent = self.path.parent().unwrap_or_else(|| Path::new("."));
        fs::create_dir_all(parent).map_err(|error| {
            VaultRegistryError::Storage(format!(
                "could not create Vault registry directory '{}': {error}",
                parent.display()
            ))
        })?;
        let encoded = serde_json::to_vec_pretty(&StoredVaultRegistry {
            schema_version: snapshot.schema_version,
            revision: snapshot.revision,
            vaults: snapshot.vaults.clone(),
        })
        .map_err(|error| {
            VaultRegistryError::Storage(format!("could not encode Vault registry: {error}"))
        })?;
        let temporary = temporary_path(&self.path)?;
        let result = (|| {
            let mut file = create_private_file(&temporary)?;
            file.write_all(&encoded).map_err(|error| {
                VaultRegistryError::Storage(format!("could not write Vault registry: {error}"))
            })?;
            file.write_all(b"\n").map_err(|error| {
                VaultRegistryError::Storage(format!("could not finalize Vault registry: {error}"))
            })?;
            file.sync_all().map_err(|error| {
                VaultRegistryError::Storage(format!("could not sync Vault registry: {error}"))
            })?;
            fs::rename(&temporary, &self.path).map_err(|error| {
                VaultRegistryError::Storage(format!(
                    "could not replace Vault registry '{}': {error}",
                    self.path.display()
                ))
            })?;
            sync_directory(parent)
        })();
        if result.is_err() {
            let _ = fs::remove_file(&temporary);
        }
        result
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    fn corrupt(&self, detail: impl fmt::Display) -> VaultRegistryRecovery {
        VaultRegistryRecovery {
            kind: VaultRegistryRecoveryKind::Corrupt,
            message: format!(
                "Vault registry '{}' is corrupt ({detail}). Restore a known-good backup or move the file aside explicitly; Hatchdoor will not overwrite it.",
                self.path.display()
            ),
        }
    }
}

fn write_lock_for(path: &Path) -> Arc<Mutex<()>> {
    static WRITE_LOCKS: OnceLock<Mutex<BTreeMap<PathBuf, Arc<Mutex<()>>>>> = OnceLock::new();

    let path = normalized_absolute_path(path);
    let mut locks = WRITE_LOCKS
        .get_or_init(|| Mutex::new(BTreeMap::new()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    locks
        .entry(path)
        .or_insert_with(|| Arc::new(Mutex::new(())))
        .clone()
}

fn normalized_absolute_path(path: &Path) -> PathBuf {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .map(|directory| directory.join(path))
            .unwrap_or_else(|_| path.to_path_buf())
    };
    let mut normalized = PathBuf::new();
    for component in absolute.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                normalized.pop();
            }
            _ => normalized.push(component.as_os_str()),
        }
    }
    normalized
}

fn temporary_path(path: &Path) -> Result<PathBuf, VaultRegistryError> {
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| {
            VaultRegistryError::Storage(format!(
                "Vault registry path '{}' has no file name",
                path.display()
            ))
        })?;
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    for nonce in 0..100_u32 {
        let candidate = parent.join(format!(".{file_name}.{}.{}.tmp", std::process::id(), nonce));
        if !candidate.exists() {
            return Ok(candidate);
        }
    }
    Err(VaultRegistryError::Storage(format!(
        "could not create a unique temporary Vault registry beside '{}'",
        path.display()
    )))
}

#[cfg(unix)]
fn create_private_file(path: &Path) -> Result<std::fs::File, VaultRegistryError> {
    use std::os::unix::fs::OpenOptionsExt;

    OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
        .map_err(|error| {
            VaultRegistryError::Storage(format!(
                "could not create temporary Vault registry '{}': {error}",
                path.display()
            ))
        })
}

#[cfg(not(unix))]
fn create_private_file(path: &Path) -> Result<std::fs::File, VaultRegistryError> {
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|error| {
            VaultRegistryError::Storage(format!(
                "could not create temporary Vault registry '{}': {error}",
                path.display()
            ))
        })
}

#[cfg(unix)]
fn sync_directory(path: &Path) -> Result<(), VaultRegistryError> {
    let directory = std::fs::File::open(path).map_err(|error| {
        VaultRegistryError::Storage(format!(
            "could not open Vault registry directory '{}': {error}",
            path.display()
        ))
    })?;
    directory.sync_all().map_err(|error| {
        VaultRegistryError::Storage(format!(
            "could not sync Vault registry directory '{}': {error}",
            path.display()
        ))
    })
}

#[cfg(not(unix))]
fn sync_directory(_path: &Path) -> Result<(), VaultRegistryError> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Barrier};

    use tempfile::tempdir;

    use super::{
        DEFAULT_VAULT_REGISTRY_PATH, REGISTRY_SCHEMA_VERSION, VaultId, VaultRecord,
        VaultRegistryError, VaultRegistryRecoveryKind, VaultRegistryState, VaultRegistryStore,
    };

    #[test]
    fn absent_registry_is_zero_vault_without_creating_a_file() {
        let directory = tempdir().expect("temporary directory");
        let path = directory.path().join("vaults.json");
        let store = VaultRegistryStore::new(&path);

        let state = store.load().expect("load absent registry");
        let VaultRegistryState::Ready(snapshot) = state else {
            panic!("absent registry entered recovery");
        };

        assert_eq!(snapshot.schema_version(), REGISTRY_SCHEMA_VERSION);
        assert_eq!(snapshot.revision(), 0);
        assert_eq!(snapshot.vault_ids().count(), 0);
        assert!(!path.exists(), "zero-Vault load created the registry");
    }

    #[test]
    fn default_store_uses_the_authoritative_instance_state_path() {
        assert_eq!(
            VaultRegistryStore::at_default_path().path(),
            std::path::Path::new(DEFAULT_VAULT_REGISTRY_PATH)
        );
        assert_eq!(DEFAULT_VAULT_REGISTRY_PATH, "/data/state/vaults.json");
    }

    #[test]
    fn generated_vault_ids_are_canonical_random_uuids() {
        let id = VaultId::generate().expect("generate Vault ID");
        let encoded = id.to_string();

        assert_eq!(encoded.len(), 36);
        assert_eq!(&encoded[8..9], "-");
        assert_eq!(&encoded[13..14], "-");
        assert_eq!(&encoded[18..19], "-");
        assert_eq!(&encoded[23..24], "-");
        assert_eq!(&encoded[14..15], "4", "UUID version");
        assert!(matches!(&encoded[19..20], "8" | "9" | "a" | "b"));
        assert_eq!(VaultId::from_str(&encoded).expect("parse generated ID"), id);
    }

    #[test]
    fn vault_ids_reject_noncanonical_or_invalid_values() {
        let uppercase = "018F47A0-7768-4D0C-8DA3-5AA28D1C31C7";
        let wrong_version = "018f47a0-7768-7d0c-8da3-5aa28d1c31c7";

        assert!(VaultId::from_str(uppercase).is_err());
        assert!(VaultId::from_str(wrong_version).is_err());
        assert!(VaultId::from_str("not-a-uuid").is_err());
    }

    #[test]
    fn committed_registry_round_trips_ids_and_monotonic_revision() {
        let directory = tempdir().expect("temporary directory");
        let path = directory.path().join("state").join("vaults.json");
        let store = VaultRegistryStore::new(&path);
        let id = VaultId::from_str("018f47a0-7768-4d0c-8da3-5aa28d1c31c7").expect("known Vault ID");

        let committed = store
            .commit(0, [(id, VaultRecord::empty())])
            .expect("commit registry");

        assert_eq!(committed.revision(), 1);
        assert_eq!(committed.vault_ids().collect::<Vec<_>>(), vec![id]);
        let encoded = std::fs::read_to_string(&path).expect("read registry");
        assert_eq!(
            encoded,
            concat!(
                "{\n",
                "  \"schema_version\": 1,\n",
                "  \"revision\": 1,\n",
                "  \"vaults\": {\n",
                "    \"018f47a0-7768-4d0c-8da3-5aa28d1c31c7\": {}\n",
                "  }\n",
                "}\n"
            )
        );

        let VaultRegistryState::Ready(restarted) = store.load().expect("reload registry") else {
            panic!("valid registry entered recovery");
        };
        assert_eq!(restarted, committed);
    }

    #[cfg(unix)]
    #[test]
    fn committed_registry_is_private_to_its_owner() {
        use std::os::unix::fs::PermissionsExt;

        let directory = tempdir().expect("temporary directory");
        let path = directory.path().join("vaults.json");
        let store = VaultRegistryStore::new(&path);

        store.commit(0, []).expect("commit empty registry");

        let mode = std::fs::metadata(path)
            .expect("registry metadata")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600);
    }

    #[test]
    fn stale_revision_is_rejected_without_changing_the_registry() {
        let directory = tempdir().expect("temporary directory");
        let path = directory.path().join("vaults.json");
        let store = VaultRegistryStore::new(&path);
        let initial = store.commit(0, []).expect("initial commit");
        let before = std::fs::read(&path).expect("registry before conflict");

        let error = store.commit(0, []).expect_err("stale commit accepted");

        assert_eq!(
            error,
            VaultRegistryError::RevisionConflict {
                expected: 0,
                actual: initial.revision(),
            }
        );
        assert_eq!(
            std::fs::read(path).expect("registry after conflict"),
            before
        );
    }

    #[test]
    fn corrupt_registry_enters_recovery_and_is_never_overwritten() {
        let directory = tempdir().expect("temporary directory");
        let path = directory.path().join("vaults.json");
        let original = b"{ definitely not valid json";
        std::fs::write(&path, original).expect("write corrupt registry");
        let store = VaultRegistryStore::new(&path);

        let VaultRegistryState::Recovery(recovery) = store.load().expect("load registry") else {
            panic!("corrupt registry was treated as ready");
        };

        assert_eq!(recovery.kind(), VaultRegistryRecoveryKind::Corrupt);
        assert!(recovery.message().contains("corrupt"));
        assert!(recovery.message().contains("will not overwrite"));
        assert_eq!(
            store.commit(0, []).expect_err("corrupt file overwritten"),
            VaultRegistryError::RecoveryRequired
        );
        assert_eq!(std::fs::read(path).expect("retained registry"), original);
    }

    #[test]
    fn future_schema_enters_recovery_with_upgrade_guidance() {
        let directory = tempdir().expect("temporary directory");
        let path = directory.path().join("vaults.json");
        let original = br#"{"schema_version":2,"revision":41,"vaults":{}}"#;
        std::fs::write(&path, original).expect("write future registry");
        let store = VaultRegistryStore::new(&path);

        let VaultRegistryState::Recovery(recovery) = store.load().expect("load registry") else {
            panic!("future registry was treated as ready");
        };

        assert_eq!(
            recovery.kind(),
            VaultRegistryRecoveryKind::FutureSchema {
                found: 2,
                supported: REGISTRY_SCHEMA_VERSION,
            }
        );
        assert!(recovery.message().contains("Upgrade Hatchdoor"));
        assert!(recovery.message().contains("will not overwrite"));
        assert_eq!(std::fs::read(path).expect("retained registry"), original);
    }

    #[test]
    fn current_schema_with_unknown_record_fields_is_recoverable_corruption() {
        let directory = tempdir().expect("temporary directory");
        let path = directory.path().join("vaults.json");
        let original = br#"{
            "schema_version": 1,
            "revision": 3,
            "vaults": {
                "018f47a0-7768-4d0c-8da3-5aa28d1c31c7": {"name": "not-yet-supported"}
            }
        }"#;
        std::fs::write(&path, original).expect("write incompatible registry");
        let store = VaultRegistryStore::new(&path);

        let VaultRegistryState::Recovery(recovery) = store.load().expect("load registry") else {
            panic!("unsupported record shape was accepted");
        };

        assert_eq!(recovery.kind(), VaultRegistryRecoveryKind::Corrupt);
        assert_eq!(std::fs::read(path).expect("retained registry"), original);
    }

    #[test]
    fn concurrent_commits_serialize_and_reject_one_stale_writer() {
        let directory = tempdir().expect("temporary directory");
        let store = Arc::new(VaultRegistryStore::new(
            directory.path().join("vaults.json"),
        ));
        store.commit(0, []).expect("initial commit");
        let barrier = Arc::new(Barrier::new(3));

        let writers = [
            "018f47a0-7768-4d0c-8da3-5aa28d1c31c7",
            "b676d7c4-ca1c-4c92-813f-b47b14a5192d",
        ]
        .map(|value| {
            let store = store.clone();
            let barrier = barrier.clone();
            let id = VaultId::from_str(value).expect("known Vault ID");
            std::thread::spawn(move || {
                barrier.wait();
                store.commit(1, [(id, VaultRecord::empty())])
            })
        });

        barrier.wait();
        let results = writers.map(|writer| writer.join().expect("writer thread"));
        assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
        assert_eq!(
            results
                .iter()
                .filter(|result| {
                    matches!(
                        result,
                        Err(VaultRegistryError::RevisionConflict {
                            expected: 1,
                            actual: 2
                        })
                    )
                })
                .count(),
            1
        );

        let VaultRegistryState::Ready(snapshot) = store.load().expect("load committed registry")
        else {
            panic!("committed registry entered recovery");
        };
        assert_eq!(snapshot.revision(), 2);
        assert_eq!(snapshot.vault_ids().count(), 1);
    }

    #[test]
    fn independently_constructed_stores_serialize_writes_to_the_same_path() {
        let directory = tempdir().expect("temporary directory");
        let path = directory.path().join("vaults.json");
        let first = Arc::new(VaultRegistryStore::new(&path));
        let second = Arc::new(VaultRegistryStore::new(&path));
        first.commit(0, []).expect("initial commit");
        let records = (0..1024)
            .map(|_| {
                (
                    VaultId::generate().expect("generate Vault ID"),
                    VaultRecord::empty(),
                )
            })
            .collect::<Vec<_>>();
        let barrier = Arc::new(Barrier::new(3));

        let writers = [first, second].map(|store| {
            let barrier = barrier.clone();
            let records = records.clone();
            std::thread::spawn(move || {
                barrier.wait();
                store.commit(1, records)
            })
        });

        barrier.wait();
        let results = writers.map(|writer| writer.join().expect("writer thread"));
        assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
        assert_eq!(
            results
                .iter()
                .filter(|result| {
                    matches!(
                        result,
                        Err(VaultRegistryError::RevisionConflict {
                            expected: 1,
                            actual: 2
                        })
                    )
                })
                .count(),
            1
        );
    }

    #[test]
    fn replacement_never_exposes_partial_json_to_readers() {
        let directory = tempdir().expect("temporary directory");
        let path = directory.path().join("vaults.json");
        let store = Arc::new(VaultRegistryStore::new(&path));
        let vaults = (0..128)
            .map(|_| {
                (
                    VaultId::generate().expect("generate Vault ID"),
                    VaultRecord::empty(),
                )
            })
            .collect::<Vec<_>>();
        store.commit(0, vaults.clone()).expect("initial registry");
        let started = Arc::new(Barrier::new(2));
        let finished = Arc::new(AtomicBool::new(false));

        let writer = {
            let store = store.clone();
            let started = started.clone();
            let finished = finished.clone();
            std::thread::spawn(move || {
                started.wait();
                for expected in 1..=12 {
                    let records = if expected % 2 == 0 {
                        vaults.clone()
                    } else {
                        Vec::new()
                    };
                    store.commit(expected, records).expect("atomic commit");
                }
                finished.store(true, Ordering::Release);
            })
        };

        started.wait();
        let mut observations = 0;
        while !finished.load(Ordering::Acquire) || observations < 64 {
            let encoded = std::fs::read(&path).expect("read while replacing");
            let value: serde_json::Value =
                serde_json::from_slice(&encoded).expect("complete registry JSON");
            assert_eq!(value["schema_version"], REGISTRY_SCHEMA_VERSION);
            assert!(value["revision"].as_u64().is_some());
            assert!(matches!(
                value["vaults"].as_object().map(|vaults| vaults.len()),
                Some(0 | 128)
            ));
            observations += 1;
        }
        writer.join().expect("writer thread");

        let temporary_files = std::fs::read_dir(directory.path())
            .expect("registry directory")
            .filter_map(Result::ok)
            .filter(|entry| entry.file_name().to_string_lossy().ends_with(".tmp"))
            .count();
        assert_eq!(temporary_files, 0);
    }
}
