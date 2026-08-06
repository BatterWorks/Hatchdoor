//! Authoritative, revisioned persistence and lifecycle management for Vault definitions.
//!
//! This boundary owns durable identity, tagged source definitions, validation,
//! credential redaction, optimistic lifecycle operations, atomicity, permissions,
//! and recovery behavior. Runtime reconciliation, Git lifecycle, migration, and
//! transport adapters remain outside this module.

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

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum VaultSource {
    Local {
        path: PathBuf,
    },
    ExistingGit {
        repository_path: PathBuf,
        repository_url: Option<String>,
        branch: Option<String>,
        vault_subdirectory: Option<PathBuf>,
        mode: VaultGitMode,
    },
    ManagedGit {
        repository_url: String,
        branch: Option<String>,
        vault_subdirectory: Option<PathBuf>,
        mode: VaultGitMode,
    },
}

impl fmt::Debug for VaultSource {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Local { path } => formatter.debug_struct("Local").field("path", path).finish(),
            Self::ExistingGit {
                repository_path,
                repository_url,
                branch,
                vault_subdirectory,
                mode,
            } => formatter
                .debug_struct("ExistingGit")
                .field("repository_path", repository_path)
                .field(
                    "repository_url",
                    &repository_url.as_ref().map(|_| "[REDACTED]"),
                )
                .field("branch", branch)
                .field("vault_subdirectory", vault_subdirectory)
                .field("mode", mode)
                .finish(),
            Self::ManagedGit {
                branch,
                vault_subdirectory,
                mode,
                ..
            } => formatter
                .debug_struct("ManagedGit")
                .field("repository_url", &"[REDACTED]")
                .field("branch", branch)
                .field("vault_subdirectory", vault_subdirectory)
                .field("mode", mode)
                .finish(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VaultGitMode {
    LocalHistory,
    PullOnly,
    TwoWay,
}

#[derive(Clone, PartialEq, Eq)]
pub struct HttpsCredentials {
    pub username: String,
    pub token: String,
}

impl fmt::Debug for HttpsCredentials {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("HttpsCredentials")
            .field("username", &"[REDACTED]")
            .field("token", &"[REDACTED]")
            .finish()
    }
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredHttpsCredentials {
    username: String,
    token: String,
}

impl fmt::Debug for StoredHttpsCredentials {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("StoredHttpsCredentials")
            .field("username", &"[REDACTED]")
            .field("token", &"[REDACTED]")
            .finish()
    }
}

impl From<HttpsCredentials> for StoredHttpsCredentials {
    fn from(credentials: HttpsCredentials) -> Self {
        Self {
            username: credentials.username,
            token: credentials.token,
        }
    }
}

pub struct NewVaultDefinition {
    pub name: String,
    pub enabled: bool,
    pub source: VaultSource,
    pub exclude_patterns: Vec<String>,
    pub https_credentials: Option<HttpsCredentials>,
}

#[derive(Debug)]
pub enum HttpsCredentialUpdate {
    Keep,
    Remove,
    Replace(HttpsCredentials),
}

pub struct VaultDefinitionEdit {
    pub name: String,
    pub source: VaultSource,
    pub exclude_patterns: Vec<String>,
    pub https_credentials: HttpsCredentialUpdate,
    pub confirm_identity_change: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VaultDefinition {
    vault_id: VaultId,
    name: String,
    enabled: bool,
    source: VaultSource,
    exclude_patterns: Vec<String>,
    credential_configured: bool,
}

impl VaultDefinition {
    pub fn vault_id(&self) -> VaultId {
        self.vault_id
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn enabled(&self) -> bool {
        self.enabled
    }

    pub fn source(&self) -> &VaultSource {
        &self.source
    }

    pub fn exclude_patterns(&self) -> &[String] {
        &self.exclude_patterns
    }

    pub fn credential_configured(&self) -> bool {
        self.credential_configured
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct VaultRecord {
    name: String,
    enabled: bool,
    source: VaultSource,
    exclude_patterns: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    https_credentials: Option<StoredHttpsCredentials>,
}

impl VaultRecord {
    fn redacted(&self, vault_id: VaultId) -> VaultDefinition {
        VaultDefinition {
            vault_id,
            name: self.name.clone(),
            enabled: self.enabled,
            source: self.source.clone(),
            exclude_patterns: self.exclude_patterns.clone(),
            credential_configured: self.https_credentials.is_some(),
        }
    }

    #[cfg(test)]
    fn empty(vault_id: VaultId) -> Self {
        Self {
            name: format!("Test Vault {vault_id}"),
            enabled: true,
            source: VaultSource::Local {
                path: PathBuf::from("/tmp/test-vault").join(vault_id.to_string()),
            },
            exclude_patterns: Vec::new(),
            https_credentials: None,
        }
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

    pub fn definitions(&self) -> impl Iterator<Item = VaultDefinition> + '_ {
        self.vaults
            .iter()
            .map(|(vault_id, record)| record.redacted(*vault_id))
    }

    pub fn definition(&self, vault_id: VaultId) -> Option<VaultDefinition> {
        self.vaults
            .get(&vault_id)
            .map(|record| record.redacted(vault_id))
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum VaultDefinitionError {
    InvalidName,
    InvalidExclusionPattern,
    DuplicateName,
    PathOverlap,
    VaultNotFound,
    IdentityChangeRequiresDisabled,
    IdentityChangeRequiresConfirmation,
    InvalidSource(String),
}

impl fmt::Display for VaultDefinitionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidName => formatter.write_str("Vault name must be non-empty"),
            Self::InvalidExclusionPattern => {
                formatter.write_str("Vault exclusion patterns must not contain control characters")
            }
            Self::DuplicateName => {
                formatter.write_str("Vault name is already used by another definition")
            }
            Self::PathOverlap => formatter.write_str(
                "Vault path overlaps another connected Vault, including disabled definitions",
            ),
            Self::VaultNotFound => formatter.write_str("Vault definition was not found"),
            Self::IdentityChangeRequiresDisabled => formatter.write_str(
                "Vault source, repository, branch, subdirectory, or location may change only while disabled",
            ),
            Self::IdentityChangeRequiresConfirmation => formatter.write_str(
                "Vault identity-bearing changes require explicit confirmation that it is the same logical Vault",
            ),
            Self::InvalidSource(message) => formatter.write_str(message),
        }
    }
}

impl std::error::Error for VaultDefinitionError {}

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
    InvalidDefinition(VaultDefinitionError),
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
            Self::InvalidDefinition(error) => error.fmt(formatter),
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

    pub fn add(
        &self,
        expected_revision: u64,
        definition: NewVaultDefinition,
    ) -> Result<VaultRegistrySnapshot, VaultRegistryError> {
        let VaultRegistryState::Ready(current) = self.load()? else {
            return Err(VaultRegistryError::RecoveryRequired);
        };
        let name = normalize_vault_name(definition.name)?;
        if current
            .vaults
            .values()
            .any(|record| vault_name_key(&record.name) == vault_name_key(&name))
        {
            return Err(VaultRegistryError::InvalidDefinition(
                VaultDefinitionError::DuplicateName,
            ));
        }
        let source = normalize_source(definition.source)?;
        let vault_id = loop {
            let candidate = VaultId::generate().map_err(|error| {
                VaultRegistryError::Storage(format!("could not generate Vault ID: {error}"))
            })?;
            if !current.vaults.contains_key(&candidate) {
                break candidate;
            }
        };
        let vault_path = canonical_or_normalized(&self.source_vault_path(vault_id, &source));
        if current.vaults.iter().any(|(existing_id, record)| {
            let existing =
                canonical_or_normalized(&self.source_vault_path(*existing_id, &record.source));
            paths_overlap(&vault_path, &existing)
        }) {
            return Err(VaultRegistryError::InvalidDefinition(
                VaultDefinitionError::PathOverlap,
            ));
        }
        let exclude_patterns = normalize_exclude_patterns(definition.exclude_patterns)?;
        let mut vaults = current.vaults;
        let https_credentials = normalize_credentials(&source, definition.https_credentials)?;
        vaults.insert(
            vault_id,
            VaultRecord {
                name,
                enabled: definition.enabled,
                source,
                exclude_patterns,
                https_credentials,
            },
        );
        self.commit(expected_revision, vaults)
    }

    pub fn edit(
        &self,
        expected_revision: u64,
        vault_id: VaultId,
        edit: VaultDefinitionEdit,
    ) -> Result<VaultRegistrySnapshot, VaultRegistryError> {
        let VaultRegistryState::Ready(current) = self.load()? else {
            return Err(VaultRegistryError::RecoveryRequired);
        };
        ensure_revision(&current, expected_revision)?;
        let Some(existing) = current.vaults.get(&vault_id).cloned() else {
            return Err(VaultRegistryError::InvalidDefinition(
                VaultDefinitionError::VaultNotFound,
            ));
        };
        let name = normalize_vault_name(edit.name)?;
        if current.vaults.iter().any(|(other_id, record)| {
            *other_id != vault_id && vault_name_key(&record.name) == vault_name_key(&name)
        }) {
            return Err(VaultRegistryError::InvalidDefinition(
                VaultDefinitionError::DuplicateName,
            ));
        }
        let source = normalize_structural_source(edit.source)?;
        let source = if same_source_identity(&existing.source, &source) {
            source
        } else {
            normalize_source(source)?
        };
        let identity_changed = !same_source_identity(&existing.source, &source);
        if identity_changed && existing.enabled {
            return Err(VaultRegistryError::InvalidDefinition(
                VaultDefinitionError::IdentityChangeRequiresDisabled,
            ));
        }
        if identity_changed && !edit.confirm_identity_change {
            return Err(VaultRegistryError::InvalidDefinition(
                VaultDefinitionError::IdentityChangeRequiresConfirmation,
            ));
        }
        let vault_path = canonical_or_normalized(&self.source_vault_path(vault_id, &source));
        if current.vaults.iter().any(|(other_id, record)| {
            if *other_id == vault_id {
                return false;
            }
            let existing_path =
                canonical_or_normalized(&self.source_vault_path(*other_id, &record.source));
            paths_overlap(&vault_path, &existing_path)
        }) {
            return Err(VaultRegistryError::InvalidDefinition(
                VaultDefinitionError::PathOverlap,
            ));
        }
        let https_credentials = match edit.https_credentials {
            HttpsCredentialUpdate::Keep
                if source_is_remote_backed(&source) && !identity_changed =>
            {
                existing.https_credentials
            }
            HttpsCredentialUpdate::Keep | HttpsCredentialUpdate::Remove => None,
            HttpsCredentialUpdate::Replace(credentials) => {
                normalize_credentials(&source, Some(credentials))?
            }
        };
        let mut vaults = current.vaults;
        vaults.insert(
            vault_id,
            VaultRecord {
                name,
                enabled: existing.enabled,
                source,
                exclude_patterns: normalize_exclude_patterns(edit.exclude_patterns)?,
                https_credentials,
            },
        );
        self.commit(expected_revision, vaults)
    }

    pub fn enable(
        &self,
        expected_revision: u64,
        vault_id: VaultId,
    ) -> Result<VaultRegistrySnapshot, VaultRegistryError> {
        self.set_enabled(expected_revision, vault_id, true)
    }

    pub fn disable(
        &self,
        expected_revision: u64,
        vault_id: VaultId,
    ) -> Result<VaultRegistrySnapshot, VaultRegistryError> {
        self.set_enabled(expected_revision, vault_id, false)
    }

    pub fn disconnect(
        &self,
        expected_revision: u64,
        vault_id: VaultId,
    ) -> Result<VaultRegistrySnapshot, VaultRegistryError> {
        let VaultRegistryState::Ready(current) = self.load()? else {
            return Err(VaultRegistryError::RecoveryRequired);
        };
        ensure_revision(&current, expected_revision)?;
        let mut vaults = current.vaults;
        if vaults.remove(&vault_id).is_none() {
            return Err(VaultRegistryError::InvalidDefinition(
                VaultDefinitionError::VaultNotFound,
            ));
        }
        self.commit(expected_revision, vaults)
    }

    fn set_enabled(
        &self,
        expected_revision: u64,
        vault_id: VaultId,
        enabled: bool,
    ) -> Result<VaultRegistrySnapshot, VaultRegistryError> {
        let VaultRegistryState::Ready(current) = self.load()? else {
            return Err(VaultRegistryError::RecoveryRequired);
        };
        ensure_revision(&current, expected_revision)?;
        let mut vaults = current.vaults;
        let Some(existing) = vaults.get(&vault_id) else {
            return Err(VaultRegistryError::InvalidDefinition(
                VaultDefinitionError::VaultNotFound,
            ));
        };
        if existing.enabled == enabled {
            return Ok(VaultRegistrySnapshot {
                schema_version: REGISTRY_SCHEMA_VERSION,
                revision: expected_revision,
                vaults,
            });
        }
        if enabled {
            normalize_source(existing.source.clone())?;
        }
        let record = vaults
            .get_mut(&vault_id)
            .expect("record checked before enable transition");
        record.enabled = enabled;
        self.commit(expected_revision, vaults)
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
            Err(_) => {
                return Ok(VaultRegistryState::Recovery(self.corrupt("invalid JSON")));
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
            Err(_) => {
                return Ok(VaultRegistryState::Recovery(
                    self.corrupt("invalid registry shape"),
                ));
            }
        };
        if let Err(detail) = self.validate_stored_definitions(&stored.vaults) {
            return Ok(VaultRegistryState::Recovery(self.corrupt(detail)));
        }
        Ok(VaultRegistryState::Ready(VaultRegistrySnapshot {
            schema_version: stored.schema_version,
            revision: stored.revision,
            vaults: stored.vaults,
        }))
    }

    fn commit(
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

    fn validate_stored_definitions(
        &self,
        vaults: &BTreeMap<VaultId, VaultRecord>,
    ) -> Result<(), &'static str> {
        validate_stored_names(vaults)?;
        for record in vaults.values() {
            if record.exclude_patterns.iter().any(|pattern| {
                pattern.is_empty()
                    || pattern.trim() != pattern
                    || pattern.chars().any(char::is_control)
            }) {
                return Err("Vault definition contains invalid exclusion patterns");
            }
            if !stored_source_is_valid(&record.source) {
                return Err("Vault definition contains an invalid or credential-bearing source");
            }
            if let Some(credentials) = &record.https_credentials
                && (!source_is_remote_backed(&record.source)
                    || credentials.username.is_empty()
                    || credentials.token.is_empty()
                    || credentials.username.trim() != credentials.username
                    || credentials.token.trim() != credentials.token)
            {
                return Err("Vault definition contains invalid HTTPS credentials");
            }
        }
        let definitions = vaults.iter().collect::<Vec<_>>();
        for (index, (vault_id, record)) in definitions.iter().enumerate() {
            let path = canonical_or_normalized(&self.source_vault_path(**vault_id, &record.source));
            for (other_id, other) in definitions.iter().skip(index + 1) {
                let other_path =
                    canonical_or_normalized(&self.source_vault_path(**other_id, &other.source));
                if paths_overlap(&path, &other_path) {
                    return Err("Vault definitions contain overlapping canonical paths");
                }
            }
        }
        Ok(())
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

    fn source_vault_path(&self, vault_id: VaultId, source: &VaultSource) -> PathBuf {
        match source {
            VaultSource::Local { path } => path.clone(),
            VaultSource::ExistingGit {
                repository_path,
                vault_subdirectory,
                ..
            } => vault_subdirectory.as_ref().map_or_else(
                || repository_path.clone(),
                |subdirectory| repository_path.join(subdirectory),
            ),
            VaultSource::ManagedGit {
                vault_subdirectory, ..
            } => {
                let state_directory = self.path.parent().unwrap_or_else(|| Path::new("."));
                let repository = state_directory
                    .join("vaults")
                    .join(vault_id.to_string())
                    .join("repository");
                vault_subdirectory
                    .as_ref()
                    .map_or(repository.clone(), |subdirectory| {
                        repository.join(subdirectory)
                    })
            }
        }
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

fn normalize_vault_name(name: String) -> Result<String, VaultRegistryError> {
    let name = name.trim().to_string();
    if name.is_empty() || name.chars().any(char::is_control) {
        return Err(VaultRegistryError::InvalidDefinition(
            VaultDefinitionError::InvalidName,
        ));
    }
    Ok(name)
}

fn vault_name_key(name: &str) -> String {
    name.to_uppercase()
}

fn ensure_revision(
    snapshot: &VaultRegistrySnapshot,
    expected_revision: u64,
) -> Result<(), VaultRegistryError> {
    if snapshot.revision != expected_revision {
        return Err(VaultRegistryError::RevisionConflict {
            expected: expected_revision,
            actual: snapshot.revision,
        });
    }
    Ok(())
}

fn normalize_exclude_patterns(patterns: Vec<String>) -> Result<Vec<String>, VaultRegistryError> {
    let patterns = patterns
        .into_iter()
        .map(|pattern| pattern.trim().to_string())
        .filter(|pattern| !pattern.is_empty())
        .collect::<Vec<_>>();
    if patterns
        .iter()
        .any(|pattern| pattern.chars().any(char::is_control))
    {
        return Err(VaultRegistryError::InvalidDefinition(
            VaultDefinitionError::InvalidExclusionPattern,
        ));
    }
    Ok(patterns)
}

fn normalize_optional_branch(branch: Option<String>) -> Result<Option<String>, VaultRegistryError> {
    let branch = branch
        .map(|branch| branch.trim().to_string())
        .filter(|branch| !branch.is_empty());
    if branch
        .as_ref()
        .is_some_and(|branch| branch.chars().any(char::is_control))
    {
        return Err(invalid_source(
            "Git branch must not contain control characters",
        ));
    }
    Ok(branch)
}

fn same_source_identity(first: &VaultSource, second: &VaultSource) -> bool {
    match (first, second) {
        (VaultSource::Local { path: first }, VaultSource::Local { path: second }) => {
            first == second
        }
        (
            VaultSource::ExistingGit {
                repository_path: first_path,
                repository_url: first_url,
                branch: first_branch,
                vault_subdirectory: first_subdirectory,
                ..
            },
            VaultSource::ExistingGit {
                repository_path: second_path,
                repository_url: second_url,
                branch: second_branch,
                vault_subdirectory: second_subdirectory,
                ..
            },
        ) => {
            first_path == second_path
                && first_url == second_url
                && first_branch == second_branch
                && first_subdirectory == second_subdirectory
        }
        (
            VaultSource::ManagedGit {
                repository_url: first_url,
                branch: first_branch,
                vault_subdirectory: first_subdirectory,
                ..
            },
            VaultSource::ManagedGit {
                repository_url: second_url,
                branch: second_branch,
                vault_subdirectory: second_subdirectory,
                ..
            },
        ) => {
            first_url == second_url
                && first_branch == second_branch
                && first_subdirectory == second_subdirectory
        }
        _ => false,
    }
}

fn normalize_source(source: VaultSource) -> Result<VaultSource, VaultRegistryError> {
    let source = normalize_structural_source(source)?;
    match source {
        VaultSource::Local { path } => {
            let path = path.canonicalize().map_err(|error| {
                VaultRegistryError::InvalidDefinition(VaultDefinitionError::InvalidSource(format!(
                    "local Vault path '{}' is not readable: {error}",
                    path.display()
                )))
            })?;
            let metadata = fs::metadata(&path).map_err(|error| {
                VaultRegistryError::InvalidDefinition(VaultDefinitionError::InvalidSource(format!(
                    "local Vault path '{}' is not readable: {error}",
                    path.display()
                )))
            })?;
            if !metadata.is_dir() {
                return Err(VaultRegistryError::InvalidDefinition(
                    VaultDefinitionError::InvalidSource(format!(
                        "local Vault path '{}' is not a directory",
                        path.display()
                    )),
                ));
            }
            fs::read_dir(&path).map_err(|error| {
                VaultRegistryError::InvalidDefinition(VaultDefinitionError::InvalidSource(format!(
                    "local Vault path '{}' is not readable: {error}",
                    path.display()
                )))
            })?;
            Ok(VaultSource::Local { path })
        }
        VaultSource::ManagedGit {
            repository_url,
            branch,
            vault_subdirectory,
            mode,
        } => Ok(VaultSource::ManagedGit {
            repository_url,
            branch,
            vault_subdirectory,
            mode,
        }),
        VaultSource::ExistingGit {
            repository_path,
            repository_url,
            branch,
            vault_subdirectory,
            mode,
        } => {
            let repository_path = repository_path.canonicalize().map_err(|error| {
                invalid_source(&format!(
                    "existing Git location '{}' is not readable: {error}",
                    repository_path.display()
                ))
            })?;
            let repository = git2::Repository::open(&repository_path).map_err(|_| {
                invalid_source("existing Git location must be a readable working checkout")
            })?;
            if repository.is_bare() || repository.workdir().is_none() {
                return Err(invalid_source(
                    "existing Git location must be a readable working checkout",
                ));
            }
            let workdir = repository
                .workdir()
                .expect("non-bare repository checked above")
                .canonicalize()
                .map_err(|_| {
                    invalid_source("existing Git location must be a readable working checkout")
                })?;
            if workdir != repository_path {
                return Err(invalid_source(
                    "existing Git location must be the working checkout root",
                ));
            }
            let vault_path = vault_subdirectory.as_ref().map_or_else(
                || repository_path.clone(),
                |subdirectory| repository_path.join(subdirectory),
            );
            let canonical_vault_path = vault_path.canonicalize().map_err(|error| {
                invalid_source(&format!(
                    "existing Git Vault path '{}' is not readable: {error}",
                    vault_path.display()
                ))
            })?;
            if !canonical_vault_path.starts_with(&repository_path) {
                return Err(invalid_source(
                    "existing Git Vault subdirectory must stay inside its repository",
                ));
            }
            fs::read_dir(&canonical_vault_path).map_err(|error| {
                invalid_source(&format!(
                    "existing Git Vault path '{}' is not readable: {error}",
                    canonical_vault_path.display()
                ))
            })?;
            let canonical_subdirectory = canonical_vault_path
                .strip_prefix(&repository_path)
                .expect("contained Vault path checked above");
            let vault_subdirectory = if canonical_subdirectory.as_os_str().is_empty() {
                None
            } else {
                Some(canonical_subdirectory.to_path_buf())
            };
            Ok(VaultSource::ExistingGit {
                repository_path,
                repository_url,
                branch,
                vault_subdirectory,
                mode,
            })
        }
    }
}

fn normalize_structural_source(source: VaultSource) -> Result<VaultSource, VaultRegistryError> {
    match source {
        VaultSource::Local { path } => Ok(VaultSource::Local { path }),
        VaultSource::ManagedGit {
            repository_url,
            branch,
            vault_subdirectory,
            mode,
        } => {
            if mode == VaultGitMode::LocalHistory {
                return Err(invalid_source(
                    "managed Git requires pull_only or two_way mode",
                ));
            }
            Ok(VaultSource::ManagedGit {
                repository_url: normalize_https_repository_url(repository_url)?,
                branch: normalize_optional_branch(branch)?,
                vault_subdirectory: vault_subdirectory
                    .map(normalize_relative_subdirectory)
                    .transpose()?,
                mode,
            })
        }
        VaultSource::ExistingGit {
            repository_path,
            repository_url,
            branch,
            vault_subdirectory,
            mode,
        } => {
            let repository_url = match repository_url {
                Some(repository_url) => Some(normalize_https_repository_url(repository_url)?),
                None if mode == VaultGitMode::LocalHistory => None,
                None => {
                    return Err(invalid_source(
                        "pull_only and two_way existing Git modes require a repository URL",
                    ));
                }
            };
            Ok(VaultSource::ExistingGit {
                repository_path,
                repository_url,
                branch: normalize_optional_branch(branch)?,
                vault_subdirectory: vault_subdirectory
                    .map(normalize_relative_subdirectory)
                    .transpose()?,
                mode,
            })
        }
    }
}

fn normalize_credentials(
    source: &VaultSource,
    credentials: Option<HttpsCredentials>,
) -> Result<Option<StoredHttpsCredentials>, VaultRegistryError> {
    let Some(credentials) = credentials else {
        return Ok(None);
    };
    if !source_is_remote_backed(source) {
        return Err(VaultRegistryError::InvalidDefinition(
            VaultDefinitionError::InvalidSource(
                "HTTPS credentials are valid only for a remote-backed Vault".to_string(),
            ),
        ));
    }
    let username = credentials.username.trim().to_string();
    let token = credentials.token.trim().to_string();
    if username.is_empty() || token.is_empty() {
        return Err(VaultRegistryError::InvalidDefinition(
            VaultDefinitionError::InvalidSource(
                "HTTPS credential username and token must both be non-empty".to_string(),
            ),
        ));
    }
    Ok(Some(StoredHttpsCredentials { username, token }))
}

fn source_is_remote_backed(source: &VaultSource) -> bool {
    matches!(
        source,
        VaultSource::ManagedGit { .. }
            | VaultSource::ExistingGit {
                repository_url: Some(_),
                ..
            }
    )
}

fn normalize_https_repository_url(repository_url: String) -> Result<String, VaultRegistryError> {
    let repository_url = repository_url.trim().to_string();
    if !is_safe_https_repository_url(&repository_url) {
        return Err(invalid_source(
            "Git repository URL must be valid credential-free HTTPS with a repository path",
        ));
    }
    Ok(repository_url)
}

fn is_safe_https_repository_url(repository_url: &str) -> bool {
    let Some(remainder) = repository_url.strip_prefix("https://") else {
        return false;
    };
    let Some((authority, path)) = remainder.split_once('/') else {
        return false;
    };
    !(authority.is_empty()
        || authority.contains('@')
        || path.is_empty()
        || repository_url.contains(['?', '#', '\\'])
        || repository_url
            .chars()
            .any(|character| character.is_whitespace() || character.is_control()))
        && valid_https_authority(authority)
        && valid_percent_encoding(repository_url)
}

fn valid_https_authority(authority: &str) -> bool {
    if let Some(ipv6) = authority.strip_prefix('[') {
        let Some((address, suffix)) = ipv6.split_once(']') else {
            return false;
        };
        return address.parse::<std::net::Ipv6Addr>().is_ok() && valid_port_suffix(suffix);
    }

    let (host, port) = match authority.rsplit_once(':') {
        Some((host, port)) => (host, Some(port)),
        None => (authority, None),
    };
    !host.is_empty()
        && !host.contains(':')
        && host.split('.').all(|label| {
            !label.is_empty()
                && !label.starts_with('-')
                && !label.ends_with('-')
                && label
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
        })
        && port.is_none_or(valid_port)
}

fn valid_port_suffix(suffix: &str) -> bool {
    suffix.is_empty() || suffix.strip_prefix(':').is_some_and(valid_port)
}

fn valid_port(port: &str) -> bool {
    !port.is_empty()
        && port.bytes().all(|byte| byte.is_ascii_digit())
        && port.parse::<u16>().is_ok()
}

fn valid_percent_encoding(value: &str) -> bool {
    let bytes = value.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' {
            if index + 2 >= bytes.len()
                || !bytes[index + 1].is_ascii_hexdigit()
                || !bytes[index + 2].is_ascii_hexdigit()
            {
                return false;
            }
            index += 3;
        } else {
            index += 1;
        }
    }
    true
}

fn normalize_relative_subdirectory(path: PathBuf) -> Result<PathBuf, VaultRegistryError> {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Normal(value) => normalized.push(value),
            _ => {
                return Err(invalid_source(
                    "Vault subdirectory must be a contained relative path",
                ));
            }
        }
    }
    if normalized.as_os_str().is_empty() {
        return Err(invalid_source(
            "Vault subdirectory must be a contained relative path",
        ));
    }
    Ok(normalized)
}

fn invalid_source(message: &str) -> VaultRegistryError {
    VaultRegistryError::InvalidDefinition(VaultDefinitionError::InvalidSource(message.to_string()))
}

fn paths_overlap(first: &Path, second: &Path) -> bool {
    first.starts_with(second) || second.starts_with(first)
}

fn validate_stored_names(vaults: &BTreeMap<VaultId, VaultRecord>) -> Result<(), &'static str> {
    let mut names = std::collections::BTreeSet::new();
    for record in vaults.values() {
        if record.name.trim().is_empty()
            || record.name.trim() != record.name
            || record.name.chars().any(char::is_control)
        {
            return Err("Vault definition contains an invalid name");
        }
        if !names.insert(vault_name_key(&record.name)) {
            return Err("Vault definitions contain duplicate case-insensitive names");
        }
    }
    Ok(())
}

fn stored_source_is_valid(source: &VaultSource) -> bool {
    if normalize_structural_source(source.clone()).as_ref() != Ok(source) {
        return false;
    }
    match source {
        VaultSource::Local { path } => {
            path.is_absolute() && normalized_absolute_path(path) == *path
        }
        VaultSource::ExistingGit {
            repository_path, ..
        } => {
            repository_path.is_absolute()
                && normalized_absolute_path(repository_path) == *repository_path
        }
        VaultSource::ManagedGit { .. } => true,
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

fn canonical_or_normalized(path: &Path) -> PathBuf {
    let normalized = normalized_absolute_path(path);
    let mut existing_ancestor = normalized.as_path();
    let mut missing_suffix = Vec::new();

    loop {
        if let Ok(mut canonical) = existing_ancestor.canonicalize() {
            for component in missing_suffix.iter().rev() {
                canonical.push(component);
            }
            return canonical;
        }
        let Some(file_name) = existing_ancestor.file_name() else {
            return normalized;
        };
        missing_suffix.push(file_name.to_os_string());
        let Some(parent) = existing_ancestor.parent() else {
            return normalized;
        };
        existing_ancestor = parent;
    }
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
    use std::path::PathBuf;
    use std::str::FromStr;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Barrier};

    use tempfile::tempdir;

    use super::{
        DEFAULT_VAULT_REGISTRY_PATH, HttpsCredentialUpdate, HttpsCredentials, NewVaultDefinition,
        REGISTRY_SCHEMA_VERSION, VaultDefinitionEdit, VaultDefinitionError, VaultGitMode, VaultId,
        VaultRecord, VaultRegistryError, VaultRegistryRecoveryKind, VaultRegistryState,
        VaultRegistryStore, VaultSource,
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
    fn local_definition_add_round_trips_through_the_public_snapshot() {
        let directory = tempdir().expect("temporary directory");
        let vault_path = directory.path().join("notes");
        std::fs::create_dir(&vault_path).expect("create Vault directory");
        let store = VaultRegistryStore::new(directory.path().join("vaults.json"));

        let committed = store
            .add(
                0,
                NewVaultDefinition {
                    name: "Personal".to_string(),
                    enabled: true,
                    source: VaultSource::Local {
                        path: vault_path.clone(),
                    },
                    exclude_patterns: vec![" private/** ".to_string(), "".to_string()],
                    https_credentials: None,
                },
            )
            .expect("add local Vault");

        assert_eq!(committed.revision(), 1);
        let definition = committed
            .definitions()
            .next()
            .expect("committed definition");
        assert_eq!(definition.name(), "Personal");
        assert!(definition.enabled());
        assert_eq!(
            definition.source(),
            &VaultSource::Local {
                path: vault_path.canonicalize().expect("canonical Vault path")
            }
        );
        assert_eq!(definition.exclude_patterns(), &["private/**"]);
        assert!(!definition.credential_configured());

        let VaultRegistryState::Ready(restarted) = store.load().expect("reload registry") else {
            panic!("valid registry entered recovery");
        };
        assert_eq!(restarted.definitions().next(), Some(definition));
    }

    #[test]
    fn vault_names_are_unique_without_regard_to_case() {
        let directory = tempdir().expect("temporary directory");
        let first_path = directory.path().join("first");
        let second_path = directory.path().join("second");
        std::fs::create_dir(&first_path).expect("create first Vault");
        std::fs::create_dir(&second_path).expect("create second Vault");
        let store = VaultRegistryStore::new(directory.path().join("vaults.json"));
        store
            .add(0, local_definition("Personal", first_path, true))
            .expect("add first Vault");

        let error = store
            .add(1, local_definition("personal", second_path, true))
            .expect_err("case-insensitive duplicate name accepted");

        assert_eq!(
            error,
            VaultRegistryError::InvalidDefinition(VaultDefinitionError::DuplicateName)
        );
        let VaultRegistryState::Ready(snapshot) = store.load().expect("reload registry") else {
            panic!("valid registry entered recovery");
        };
        assert_eq!(snapshot.revision(), 1);
        assert_eq!(snapshot.definitions().count(), 1);
    }

    #[test]
    fn vault_name_uniqueness_handles_case_mappings_that_expand() {
        let directory = tempdir().expect("temporary directory");
        let first_path = directory.path().join("first");
        let second_path = directory.path().join("second");
        std::fs::create_dir(&first_path).expect("create first Vault");
        std::fs::create_dir(&second_path).expect("create second Vault");
        let store = VaultRegistryStore::new(directory.path().join("vaults.json"));
        store
            .add(0, local_definition("Straße", first_path, true))
            .expect("add first Vault");

        let error = store
            .add(1, local_definition("STRASSE", second_path, true))
            .expect_err("caseless-equivalent name accepted");

        assert_eq!(
            error,
            VaultRegistryError::InvalidDefinition(VaultDefinitionError::DuplicateName)
        );
    }

    #[test]
    fn disabled_definitions_continue_reserving_their_canonical_paths() {
        let directory = tempdir().expect("temporary directory");
        let parent = directory.path().join("notes");
        let nested = parent.join("nested");
        std::fs::create_dir_all(&nested).expect("create nested Vault paths");
        let store = VaultRegistryStore::new(directory.path().join("vaults.json"));
        store
            .add(0, local_definition("Paused", parent, false))
            .expect("add disabled Vault");

        let error = store
            .add(1, local_definition("Nested", nested, true))
            .expect_err("nested path accepted");

        assert_eq!(
            error,
            VaultRegistryError::InvalidDefinition(VaultDefinitionError::PathOverlap)
        );
        let VaultRegistryState::Ready(snapshot) = store.load().expect("reload registry") else {
            panic!("valid registry entered recovery");
        };
        assert_eq!(snapshot.revision(), 1);
    }

    #[test]
    fn managed_https_credentials_are_persisted_but_redacted_from_reads_and_debug() {
        let directory = tempdir().expect("temporary directory");
        let store = VaultRegistryStore::new(directory.path().join("vaults.json"));
        let credentials = HttpsCredentials {
            username: "git-user".to_string(),
            token: "super-secret-token".to_string(),
        };
        assert!(!format!("{credentials:?}").contains("super-secret-token"));

        let committed = store
            .add(
                0,
                NewVaultDefinition {
                    name: "Remote notes".to_string(),
                    enabled: true,
                    source: VaultSource::ManagedGit {
                        repository_url: "https://example.test/owner/notes.git".to_string(),
                        branch: Some("main".to_string()),
                        vault_subdirectory: Some(PathBuf::from("knowledge/notes")),
                        mode: VaultGitMode::PullOnly,
                    },
                    exclude_patterns: Vec::new(),
                    https_credentials: Some(credentials),
                },
            )
            .expect("add managed Vault");

        let definition = committed.definitions().next().expect("definition");
        assert!(definition.credential_configured());
        assert!(!format!("{definition:?}").contains("super-secret-token"));
        assert_eq!(
            definition.source(),
            &VaultSource::ManagedGit {
                repository_url: "https://example.test/owner/notes.git".to_string(),
                branch: Some("main".to_string()),
                vault_subdirectory: Some(PathBuf::from("knowledge/notes")),
                mode: VaultGitMode::PullOnly,
            }
        );
        let encoded = std::fs::read_to_string(store.path()).expect("persisted registry");
        assert!(encoded.contains("super-secret-token"));
        assert!(!format!("{committed:?}").contains("super-secret-token"));
    }

    #[test]
    fn existing_git_source_requires_a_readable_checkout_and_canonicalizes_its_location() {
        let directory = tempdir().expect("temporary directory");
        let repository_path = directory.path().join("repository");
        git2::Repository::init(&repository_path).expect("initialize repository");
        std::fs::create_dir(repository_path.join("notes")).expect("create Vault subdirectory");
        let store = VaultRegistryStore::new(directory.path().join("vaults.json"));

        let committed = store
            .add(
                0,
                NewVaultDefinition {
                    name: "Existing checkout".to_string(),
                    enabled: true,
                    source: VaultSource::ExistingGit {
                        repository_path: repository_path.clone(),
                        repository_url: None,
                        branch: None,
                        vault_subdirectory: Some(PathBuf::from("notes")),
                        mode: VaultGitMode::LocalHistory,
                    },
                    exclude_patterns: Vec::new(),
                    https_credentials: None,
                },
            )
            .expect("add existing checkout");

        assert_eq!(
            committed.definitions().next().expect("definition").source(),
            &VaultSource::ExistingGit {
                repository_path: repository_path
                    .canonicalize()
                    .expect("canonical repository"),
                repository_url: None,
                branch: None,
                vault_subdirectory: Some(PathBuf::from("notes")),
                mode: VaultGitMode::LocalHistory,
            }
        );

        let not_repository = directory.path().join("ordinary-directory");
        std::fs::create_dir(&not_repository).expect("create ordinary directory");
        let error = store
            .add(
                1,
                NewVaultDefinition {
                    name: "Not Git".to_string(),
                    enabled: true,
                    source: VaultSource::ExistingGit {
                        repository_path: not_repository,
                        repository_url: None,
                        branch: None,
                        vault_subdirectory: None,
                        mode: VaultGitMode::LocalHistory,
                    },
                    exclude_patterns: Vec::new(),
                    https_credentials: None,
                },
            )
            .expect_err("ordinary directory accepted as existing Git");
        assert!(matches!(
            error,
            VaultRegistryError::InvalidDefinition(VaultDefinitionError::InvalidSource(_))
        ));
    }

    #[test]
    fn existing_git_retains_remote_identity_while_git_mode_changes_normally() {
        let directory = tempdir().expect("temporary directory");
        let repository_path = directory.path().join("repository");
        git2::Repository::init(&repository_path).expect("initialize repository");
        let store = VaultRegistryStore::new(directory.path().join("vaults.json"));
        let repository_url = "https://example.test/notes.git".to_string();
        let added = store
            .add(
                0,
                NewVaultDefinition {
                    name: "Local history".to_string(),
                    enabled: true,
                    source: VaultSource::ExistingGit {
                        repository_path: repository_path.clone(),
                        repository_url: Some(repository_url.clone()),
                        branch: Some("main".to_string()),
                        vault_subdirectory: None,
                        mode: VaultGitMode::LocalHistory,
                    },
                    exclude_patterns: Vec::new(),
                    https_credentials: None,
                },
            )
            .expect("connect local history without discarding remote identity");
        let vault_id = added.definitions().next().expect("definition").vault_id();

        let edited = store
            .edit(
                1,
                vault_id,
                VaultDefinitionEdit {
                    name: "Remote enabled".to_string(),
                    source: VaultSource::ExistingGit {
                        repository_path,
                        repository_url: Some(repository_url),
                        branch: Some("main".to_string()),
                        vault_subdirectory: None,
                        mode: VaultGitMode::PullOnly,
                    },
                    exclude_patterns: Vec::new(),
                    https_credentials: HttpsCredentialUpdate::Keep,
                    confirm_identity_change: false,
                },
            )
            .expect("change only Git behavior while enabled");

        assert!(matches!(
            edited.definitions().next().expect("definition").source(),
            VaultSource::ExistingGit {
                repository_url: Some(url),
                mode: VaultGitMode::PullOnly,
                ..
            } if url == "https://example.test/notes.git"
        ));
    }

    #[cfg(unix)]
    #[test]
    fn existing_git_symlink_subdirectory_cannot_bypass_canonical_overlap_validation() {
        use std::os::unix::fs::symlink;

        let directory = tempdir().expect("temporary directory");
        let repository_path = directory.path().join("repository");
        git2::Repository::init(&repository_path).expect("initialize repository");
        let actual = repository_path.join("actual");
        std::fs::create_dir(&actual).expect("create actual Vault path");
        symlink("actual", repository_path.join("alias")).expect("create in-repository symlink");
        let store = VaultRegistryStore::new(directory.path().join("vaults.json"));
        let added = store
            .add(
                0,
                NewVaultDefinition {
                    name: "Existing checkout".to_string(),
                    enabled: false,
                    source: VaultSource::ExistingGit {
                        repository_path,
                        repository_url: None,
                        branch: None,
                        vault_subdirectory: Some(PathBuf::from("alias")),
                        mode: VaultGitMode::LocalHistory,
                    },
                    exclude_patterns: Vec::new(),
                    https_credentials: None,
                },
            )
            .expect("add existing checkout");
        assert!(matches!(
            added.definitions().next().expect("definition").source(),
            VaultSource::ExistingGit {
                vault_subdirectory: Some(path),
                ..
            } if path == std::path::Path::new("actual")
        ));

        let error = store
            .add(1, local_definition("Same files", actual, true))
            .expect_err("symlink alias bypassed path overlap");

        assert_eq!(
            error,
            VaultRegistryError::InvalidDefinition(VaultDefinitionError::PathOverlap)
        );
    }

    #[test]
    fn existing_git_source_rejects_the_repository_metadata_directory_as_its_location() {
        let directory = tempdir().expect("temporary directory");
        let repository_path = directory.path().join("repository");
        git2::Repository::init(&repository_path).expect("initialize repository");
        let path = directory.path().join("vaults.json");
        let store = VaultRegistryStore::new(&path);

        let error = store
            .add(
                0,
                NewVaultDefinition {
                    name: "Metadata is not a Vault".to_string(),
                    enabled: true,
                    source: VaultSource::ExistingGit {
                        repository_path: repository_path.join(".git"),
                        repository_url: None,
                        branch: None,
                        vault_subdirectory: None,
                        mode: VaultGitMode::LocalHistory,
                    },
                    exclude_patterns: Vec::new(),
                    https_credentials: None,
                },
            )
            .expect_err("repository metadata accepted as checkout root");

        assert!(matches!(
            error,
            VaultRegistryError::InvalidDefinition(VaultDefinitionError::InvalidSource(_))
        ));
        assert!(!path.exists());
    }

    #[test]
    fn malformed_https_repository_urls_save_nothing() {
        for repository_url in [
            "https://:bad/notes.git",
            "https://example.test/%zz/notes.git",
        ] {
            let directory = tempdir().expect("temporary directory");
            let path = directory.path().join("vaults.json");
            let store = VaultRegistryStore::new(&path);

            let error = store
                .add(
                    0,
                    NewVaultDefinition {
                        name: "Malformed remote".to_string(),
                        enabled: true,
                        source: VaultSource::ManagedGit {
                            repository_url: repository_url.to_string(),
                            branch: None,
                            vault_subdirectory: None,
                            mode: VaultGitMode::PullOnly,
                        },
                        exclude_patterns: Vec::new(),
                        https_credentials: None,
                    },
                )
                .expect_err("malformed HTTPS URL accepted");

            assert!(matches!(
                error,
                VaultRegistryError::InvalidDefinition(VaultDefinitionError::InvalidSource(_))
            ));
            assert!(!path.exists());
        }
    }

    #[test]
    fn identity_changes_require_a_disabled_definition_and_explicit_confirmation() {
        let directory = tempdir().expect("temporary directory");
        let original_path = directory.path().join("original");
        let replacement_path = directory.path().join("replacement");
        std::fs::create_dir(&original_path).expect("create original Vault");
        std::fs::create_dir(&replacement_path).expect("create replacement Vault");
        let store = VaultRegistryStore::new(directory.path().join("vaults.json"));
        let added = store
            .add(0, local_definition("Notes", original_path, true))
            .expect("add Vault");
        let vault_id = added.definitions().next().expect("definition").vault_id();

        let enabled_error = store
            .edit(
                1,
                vault_id,
                local_edit("Notes", replacement_path.clone(), true),
            )
            .expect_err("enabled identity change accepted");
        assert_eq!(
            enabled_error,
            VaultRegistryError::InvalidDefinition(
                VaultDefinitionError::IdentityChangeRequiresDisabled
            )
        );

        let disabled = store.disable(1, vault_id).expect("disable Vault");
        assert_eq!(disabled.revision(), 2);
        assert!(!disabled.definitions().next().expect("definition").enabled());

        let confirmation_error = store
            .edit(
                2,
                vault_id,
                local_edit("Notes", replacement_path.clone(), false),
            )
            .expect_err("unconfirmed identity change accepted");
        assert_eq!(
            confirmation_error,
            VaultRegistryError::InvalidDefinition(
                VaultDefinitionError::IdentityChangeRequiresConfirmation
            )
        );

        let edited = store
            .edit(
                2,
                vault_id,
                local_edit("Notes", replacement_path.clone(), true),
            )
            .expect("confirmed disabled identity change");
        let definition = edited.definitions().next().expect("definition");
        assert_eq!(definition.vault_id(), vault_id);
        assert!(!definition.enabled());
        assert_eq!(
            definition.source(),
            &VaultSource::Local {
                path: replacement_path
                    .canonicalize()
                    .expect("canonical replacement")
            }
        );
    }

    #[test]
    fn disconnect_removes_only_the_definition_and_preserves_vault_files() {
        let directory = tempdir().expect("temporary directory");
        let vault_path = directory.path().join("notes");
        std::fs::create_dir(&vault_path).expect("create Vault");
        let note_path = vault_path.join("kept.md");
        std::fs::write(&note_path, "# Kept\n").expect("write note");
        let store = VaultRegistryStore::new(directory.path().join("vaults.json"));
        let added = store
            .add(0, local_definition("Notes", vault_path.clone(), true))
            .expect("add Vault");
        let vault_id = added.definitions().next().expect("definition").vault_id();

        let disconnected = store.disconnect(1, vault_id).expect("disconnect Vault");

        assert_eq!(disconnected.revision(), 2);
        assert_eq!(disconnected.definitions().count(), 0);
        assert_eq!(
            std::fs::read_to_string(note_path).expect("preserved note"),
            "# Kept\n"
        );
        assert!(vault_path.exists());
    }

    #[test]
    fn structurally_invalid_persisted_definitions_enter_recovery_without_overwrite() {
        let directory = tempdir().expect("temporary directory");
        let path = directory.path().join("vaults.json");
        let original = br#"{
          "schema_version": 1,
          "revision": 8,
          "vaults": {
            "018f47a0-7768-4d0c-8da3-5aa28d1c31c7": {
              "name": "Personal",
              "enabled": true,
              "source": {"type": "local", "path": "/srv/first"},
              "exclude_patterns": []
            },
            "b676d7c4-ca1c-4c92-813f-b47b14a5192d": {
              "name": "personal",
              "enabled": false,
              "source": {"type": "local", "path": "/srv/second"},
              "exclude_patterns": []
            }
          }
        }"#;
        std::fs::write(&path, original).expect("write invalid registry");
        let store = VaultRegistryStore::new(&path);

        let VaultRegistryState::Recovery(recovery) = store.load().expect("load registry") else {
            panic!("invalid definitions were treated as ready");
        };

        assert_eq!(recovery.kind(), VaultRegistryRecoveryKind::Corrupt);
        assert_eq!(std::fs::read(&path).expect("preserved registry"), original);
        assert_eq!(
            store
                .disconnect(
                    8,
                    VaultId::from_str("018f47a0-7768-4d0c-8da3-5aa28d1c31c7").expect("known ID")
                )
                .expect_err("invalid registry overwritten"),
            VaultRegistryError::RecoveryRequired
        );
    }

    #[test]
    fn credential_bearing_repository_urls_enter_redacted_recovery() {
        let directory = tempdir().expect("temporary directory");
        let path = directory.path().join("vaults.json");
        let original = br#"{
          "schema_version": 1,
          "revision": 4,
          "vaults": {
            "018f47a0-7768-4d0c-8da3-5aa28d1c31c7": {
              "name": "Remote",
              "enabled": true,
              "source": {
                "type": "managed_git",
                "repository_url": "https://git-user:url-secret@example.test/notes.git",
                "branch": "main",
                "vault_subdirectory": null,
                "mode": "pull_only"
              },
              "exclude_patterns": []
            }
          }
        }"#;
        std::fs::write(&path, original).expect("write unsafe registry");
        let store = VaultRegistryStore::new(&path);

        let VaultRegistryState::Recovery(recovery) = store.load().expect("load registry") else {
            panic!("credential-bearing URL was treated as ready");
        };

        assert_eq!(recovery.kind(), VaultRegistryRecoveryKind::Corrupt);
        assert!(!recovery.message().contains("url-secret"));
        assert_eq!(std::fs::read(&path).expect("preserved registry"), original);
    }

    #[test]
    fn malformed_persisted_secret_fields_do_not_echo_values_in_recovery() {
        let directory = tempdir().expect("temporary directory");
        let path = directory.path().join("vaults.json");
        let original = br#"{
          "schema_version": 1,
          "revision": 4,
          "vaults": {
            "018f47a0-7768-4d0c-8da3-5aa28d1c31c7": {
              "name": "Remote",
              "enabled": true,
              "source": {
                "type": "managed_git",
                "repository_url": "https://example.test/notes.git",
                "branch": "main",
                "vault_subdirectory": null,
                "mode": "value-that-must-not-leak"
              },
              "exclude_patterns": []
            }
          }
        }"#;
        std::fs::write(&path, original).expect("write malformed registry");
        let store = VaultRegistryStore::new(&path);

        let VaultRegistryState::Recovery(recovery) = store.load().expect("load registry") else {
            panic!("malformed definition was treated as ready");
        };

        assert_eq!(recovery.kind(), VaultRegistryRecoveryKind::Corrupt);
        assert!(!recovery.message().contains("value-that-must-not-leak"));
        assert_eq!(std::fs::read(&path).expect("preserved registry"), original);
    }

    #[test]
    fn enabled_definitions_allow_name_mode_and_credential_changes() {
        let directory = tempdir().expect("temporary directory");
        let store = VaultRegistryStore::new(directory.path().join("vaults.json"));
        let source = VaultSource::ManagedGit {
            repository_url: "https://example.test/notes.git".to_string(),
            branch: Some("main".to_string()),
            vault_subdirectory: None,
            mode: VaultGitMode::PullOnly,
        };
        let added = store
            .add(
                0,
                NewVaultDefinition {
                    name: "Remote".to_string(),
                    enabled: true,
                    source: source.clone(),
                    exclude_patterns: Vec::new(),
                    https_credentials: Some(HttpsCredentials {
                        username: "old-user".to_string(),
                        token: "old-token".to_string(),
                    }),
                },
            )
            .expect("add remote Vault");
        let vault_id = added.definitions().next().expect("definition").vault_id();

        let edited = store
            .edit(
                1,
                vault_id,
                VaultDefinitionEdit {
                    name: "Renamed remote".to_string(),
                    source: VaultSource::ManagedGit {
                        repository_url: "https://example.test/notes.git".to_string(),
                        branch: Some("main".to_string()),
                        vault_subdirectory: None,
                        mode: VaultGitMode::TwoWay,
                    },
                    exclude_patterns: vec!["drafts/**".to_string()],
                    https_credentials: HttpsCredentialUpdate::Replace(HttpsCredentials {
                        username: "new-user".to_string(),
                        token: "new-token".to_string(),
                    }),
                    confirm_identity_change: false,
                },
            )
            .expect("edit non-identity fields");

        let definition = edited.definitions().next().expect("definition");
        assert!(definition.enabled());
        assert_eq!(definition.name(), "Renamed remote");
        assert!(definition.credential_configured());
        assert!(matches!(
            definition.source(),
            VaultSource::ManagedGit {
                mode: VaultGitMode::TwoWay,
                ..
            }
        ));
        let encoded = std::fs::read_to_string(store.path()).expect("registry");
        assert!(!encoded.contains("old-token"));
        assert!(encoded.contains("new-token"));

        let removed = store
            .edit(
                2,
                vault_id,
                VaultDefinitionEdit {
                    name: "Renamed remote".to_string(),
                    source: definition.source().clone(),
                    exclude_patterns: vec!["drafts/**".to_string()],
                    https_credentials: HttpsCredentialUpdate::Remove,
                    confirm_identity_change: false,
                },
            )
            .expect("remove credential");
        assert!(
            !removed
                .definitions()
                .next()
                .expect("definition")
                .credential_configured()
        );
        assert!(
            !std::fs::read_to_string(store.path())
                .expect("registry")
                .contains("new-token")
        );
    }

    #[test]
    fn non_identity_edits_do_not_require_the_source_to_be_currently_available() {
        let directory = tempdir().expect("temporary directory");
        let vault_path = directory.path().join("notes");
        std::fs::create_dir(&vault_path).expect("create Vault");
        let store = VaultRegistryStore::new(directory.path().join("vaults.json"));
        let added = store
            .add(
                0,
                local_definition("Mounted notes", vault_path.clone(), true),
            )
            .expect("add local Vault");
        let vault_id = added.definitions().next().expect("definition").vault_id();
        std::fs::remove_dir(&vault_path).expect("make source temporarily unavailable");

        let edited = store
            .edit(
                1,
                vault_id,
                local_edit("Renamed while unavailable", vault_path, false),
            )
            .expect("rename without changing source identity");

        assert_eq!(edited.revision(), 2);
        assert_eq!(
            edited.definitions().next().expect("definition").name(),
            "Renamed while unavailable"
        );
    }

    #[test]
    fn invalid_sources_reject_the_transaction_without_leaking_credentials() {
        let directory = tempdir().expect("temporary directory");
        let path = directory.path().join("vaults.json");
        let store = VaultRegistryStore::new(&path);
        let unsafe_source = VaultSource::ManagedGit {
            repository_url: "https://git-user:embedded-secret@example.test/notes.git".to_string(),
            branch: None,
            vault_subdirectory: None,
            mode: VaultGitMode::PullOnly,
        };
        assert!(!format!("{unsafe_source:?}").contains("embedded-secret"));
        let error = store
            .add(
                0,
                NewVaultDefinition {
                    name: "Unsafe remote".to_string(),
                    enabled: true,
                    source: unsafe_source,
                    exclude_patterns: Vec::new(),
                    https_credentials: None,
                },
            )
            .expect_err("credential-bearing URL accepted");
        assert!(!error.to_string().contains("embedded-secret"));
        assert!(!path.exists());

        let error = store
            .add(
                0,
                NewVaultDefinition {
                    name: "Escaping remote".to_string(),
                    enabled: true,
                    source: VaultSource::ManagedGit {
                        repository_url: "https://example.test/notes.git".to_string(),
                        branch: None,
                        vault_subdirectory: Some(PathBuf::from("../outside")),
                        mode: VaultGitMode::PullOnly,
                    },
                    exclude_patterns: Vec::new(),
                    https_credentials: None,
                },
            )
            .expect_err("escaping subdirectory accepted");
        assert!(matches!(
            error,
            VaultRegistryError::InvalidDefinition(VaultDefinitionError::InvalidSource(_))
        ));
        assert!(!path.exists());
    }

    #[cfg(unix)]
    #[test]
    fn readable_non_writable_local_vaults_remain_valid() {
        use std::os::unix::fs::PermissionsExt;

        let directory = tempdir().expect("temporary directory");
        let vault_path = directory.path().join("read-only-notes");
        std::fs::create_dir(&vault_path).expect("create Vault");
        std::fs::set_permissions(&vault_path, std::fs::Permissions::from_mode(0o555))
            .expect("make Vault non-writable");
        let store = VaultRegistryStore::new(directory.path().join("vaults.json"));

        let committed = store
            .add(0, local_definition("Read only", vault_path, true))
            .expect("connect readable non-writable Vault");

        assert!(
            committed
                .definitions()
                .next()
                .expect("definition")
                .enabled()
        );
    }

    #[test]
    fn enable_preserves_identity_and_increments_the_registry_revision() {
        let directory = tempdir().expect("temporary directory");
        let vault_path = directory.path().join("notes");
        std::fs::create_dir(&vault_path).expect("create Vault");
        let store = VaultRegistryStore::new(directory.path().join("vaults.json"));
        let added = store
            .add(0, local_definition("Paused", vault_path, false))
            .expect("add disabled Vault");
        let vault_id = added.definitions().next().expect("definition").vault_id();

        let enabled = store.enable(1, vault_id).expect("enable Vault");

        let definition = enabled.definitions().next().expect("definition");
        assert_eq!(enabled.revision(), 2);
        assert_eq!(definition.vault_id(), vault_id);
        assert!(definition.enabled());
    }

    #[test]
    fn enable_revalidates_a_local_source_before_saving() {
        let directory = tempdir().expect("temporary directory");
        let vault_path = directory.path().join("notes");
        std::fs::create_dir(&vault_path).expect("create Vault");
        let store = VaultRegistryStore::new(directory.path().join("vaults.json"));
        let added = store
            .add(0, local_definition("Paused", vault_path.clone(), false))
            .expect("add disabled Vault");
        let vault_id = added.definitions().next().expect("definition").vault_id();
        std::fs::remove_dir(&vault_path).expect("make source unavailable");

        let error = store
            .enable(1, vault_id)
            .expect_err("unreadable source was enabled");

        assert!(matches!(
            error,
            VaultRegistryError::InvalidDefinition(VaultDefinitionError::InvalidSource(_))
        ));
        let VaultRegistryState::Ready(snapshot) = store.load().expect("reload registry") else {
            panic!("valid registry entered recovery");
        };
        assert_eq!(snapshot.revision(), 1);
        assert!(!snapshot.definition(vault_id).expect("definition").enabled());
    }

    #[test]
    fn confirmed_repository_change_does_not_carry_an_old_credential() {
        let directory = tempdir().expect("temporary directory");
        let store = VaultRegistryStore::new(directory.path().join("vaults.json"));
        let added = store
            .add(
                0,
                NewVaultDefinition {
                    name: "Paused remote".to_string(),
                    enabled: false,
                    source: VaultSource::ManagedGit {
                        repository_url: "https://old.example.test/notes.git".to_string(),
                        branch: Some("main".to_string()),
                        vault_subdirectory: None,
                        mode: VaultGitMode::PullOnly,
                    },
                    exclude_patterns: Vec::new(),
                    https_credentials: Some(HttpsCredentials {
                        username: "old-user".to_string(),
                        token: "old-repository-token".to_string(),
                    }),
                },
            )
            .expect("add managed Vault");
        let vault_id = added.definitions().next().expect("definition").vault_id();

        let edited = store
            .edit(
                1,
                vault_id,
                VaultDefinitionEdit {
                    name: "Paused remote".to_string(),
                    source: VaultSource::ManagedGit {
                        repository_url: "https://new.example.test/notes.git".to_string(),
                        branch: Some("main".to_string()),
                        vault_subdirectory: None,
                        mode: VaultGitMode::PullOnly,
                    },
                    exclude_patterns: Vec::new(),
                    https_credentials: HttpsCredentialUpdate::Keep,
                    confirm_identity_change: true,
                },
            )
            .expect("confirm repository change");

        assert!(
            !edited
                .definitions()
                .next()
                .expect("definition")
                .credential_configured()
        );
        assert!(
            !std::fs::read_to_string(store.path())
                .expect("registry")
                .contains("old-repository-token")
        );
    }

    #[test]
    fn managed_checkout_paths_participate_in_overlap_validation() {
        let directory = tempdir().expect("temporary directory");
        let store = VaultRegistryStore::new(directory.path().join("vaults.json"));
        let added = store
            .add(
                0,
                NewVaultDefinition {
                    name: "Managed".to_string(),
                    enabled: false,
                    source: VaultSource::ManagedGit {
                        repository_url: "https://example.test/notes.git".to_string(),
                        branch: None,
                        vault_subdirectory: None,
                        mode: VaultGitMode::PullOnly,
                    },
                    exclude_patterns: Vec::new(),
                    https_credentials: None,
                },
            )
            .expect("add managed Vault");
        let vault_id = added.definitions().next().expect("definition").vault_id();
        let managed_checkout = directory
            .path()
            .join("vaults")
            .join(vault_id.to_string())
            .join("repository");
        std::fs::create_dir_all(&managed_checkout).expect("create managed checkout location");

        let error = store
            .add(
                1,
                local_definition("Overlapping local", managed_checkout, true),
            )
            .expect_err("managed checkout overlap accepted");

        assert_eq!(
            error,
            VaultRegistryError::InvalidDefinition(VaultDefinitionError::PathOverlap)
        );
    }

    #[cfg(unix)]
    #[test]
    fn absent_managed_checkout_resolves_a_symlinked_state_parent_for_overlap_checks() {
        use std::os::unix::fs::symlink;

        let directory = tempdir().expect("temporary directory");
        let real_state = directory.path().join("real-state");
        let reserved_checkouts = real_state.join("vaults");
        std::fs::create_dir_all(&reserved_checkouts).expect("create real state directory");
        let linked_state = directory.path().join("linked-state");
        symlink(&real_state, &linked_state).expect("symlink state directory");
        let store = VaultRegistryStore::new(linked_state.join("vaults.json"));
        store
            .add(
                0,
                NewVaultDefinition {
                    name: "Managed".to_string(),
                    enabled: false,
                    source: VaultSource::ManagedGit {
                        repository_url: "https://example.test/notes.git".to_string(),
                        branch: None,
                        vault_subdirectory: None,
                        mode: VaultGitMode::PullOnly,
                    },
                    exclude_patterns: Vec::new(),
                    https_credentials: None,
                },
            )
            .expect("add managed Vault before checkout acquisition");

        let error = store
            .add(
                1,
                local_definition("Overlapping local", reserved_checkouts, true),
            )
            .expect_err("symlink-aliased managed checkout parent accepted");

        assert_eq!(
            error,
            VaultRegistryError::InvalidDefinition(VaultDefinitionError::PathOverlap)
        );
    }

    fn local_definition(name: &str, path: PathBuf, enabled: bool) -> NewVaultDefinition {
        NewVaultDefinition {
            name: name.to_string(),
            enabled,
            source: VaultSource::Local { path },
            exclude_patterns: Vec::new(),
            https_credentials: None,
        }
    }

    fn local_edit(name: &str, path: PathBuf, confirm_identity_change: bool) -> VaultDefinitionEdit {
        VaultDefinitionEdit {
            name: name.to_string(),
            source: VaultSource::Local { path },
            exclude_patterns: Vec::new(),
            https_credentials: HttpsCredentialUpdate::Keep,
            confirm_identity_change,
        }
    }

    #[test]
    fn committed_registry_round_trips_ids_and_monotonic_revision() {
        let directory = tempdir().expect("temporary directory");
        let path = directory.path().join("state").join("vaults.json");
        let store = VaultRegistryStore::new(&path);
        let id = VaultId::from_str("018f47a0-7768-4d0c-8da3-5aa28d1c31c7").expect("known Vault ID");

        let committed = store
            .commit(0, [(id, VaultRecord::empty(id))])
            .expect("commit registry");

        assert_eq!(committed.revision(), 1);
        assert_eq!(committed.vault_ids().collect::<Vec<_>>(), vec![id]);
        let encoded = std::fs::read_to_string(&path).expect("read registry");
        let encoded: serde_json::Value = serde_json::from_str(&encoded).expect("registry JSON");
        assert_eq!(encoded["schema_version"], 1);
        assert_eq!(encoded["revision"], 1);
        assert_eq!(
            encoded["vaults"]["018f47a0-7768-4d0c-8da3-5aa28d1c31c7"]["name"],
            "Test Vault 018f47a0-7768-4d0c-8da3-5aa28d1c31c7"
        );
        assert_eq!(
            encoded["vaults"]["018f47a0-7768-4d0c-8da3-5aa28d1c31c7"]["source"]["type"],
            "local"
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
                store.commit(1, [(id, VaultRecord::empty(id))])
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
                let id = VaultId::generate().expect("generate Vault ID");
                (id, VaultRecord::empty(id))
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
                let id = VaultId::generate().expect("generate Vault ID");
                (id, VaultRecord::empty(id))
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
