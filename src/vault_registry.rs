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
use std::time::Duration;

use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// The on-disk `vaults.json` format version.
pub const REGISTRY_SCHEMA_VERSION: u32 = 1;
/// The authoritative Vault collection registry in persistent instance state.
pub const DEFAULT_VAULT_REGISTRY_PATH: &str = "/data/state/vaults.json";

/// Default `poll_interval_secs` for a managed-Git source: 24h, mirroring
/// `git::managed_task::DEFAULT_POLL_INTERVAL`. Kept as an independent
/// constant here rather than imported from `git::managed_task` so this
/// lower-level persistence boundary does not take on a reverse dependency
/// on the Git scheduling boundary that already consumes it (see
/// `docs/architecture/module-map.md`'s "Vault collection registry" versus
/// "Git synchronization" consumer direction) — this is the same tradeoff
/// `BACKOFF_MAX`-mirroring `MIN_MANAGED_GIT_POLL_INTERVAL_SECS` below makes.
pub(crate) const DEFAULT_MANAGED_GIT_POLL_INTERVAL_SECS: u64 = 24 * 60 * 60;

/// The shortest allowed `poll_interval_secs`: mirrors
/// `git::managed_task::BACKOFF_MAX` (1h). A shorter "normal" schedule than
/// the bound meant to throttle repeated transient failures would make the
/// bound meaningless, and a near-zero interval would turn periodic polling
/// into a busy loop.
const MIN_MANAGED_GIT_POLL_INTERVAL_SECS: u64 = 60 * 60;

fn default_managed_git_poll_interval_secs() -> u64 {
    DEFAULT_MANAGED_GIT_POLL_INTERVAL_SECS
}

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
        /// How often `ManagedGitScheduler` polls this Vault's remote absent
        /// a manual Sync/Retry or a backoff-triggering transient failure.
        /// Defaulted on deserialization so an on-disk registry written before
        /// this field existed keeps loading under the same
        /// `REGISTRY_SCHEMA_VERSION` (see `default_managed_git_poll_interval_secs`).
        #[serde(default = "default_managed_git_poll_interval_secs")]
        poll_interval_secs: u64,
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
                poll_interval_secs,
                ..
            } => formatter
                .debug_struct("ManagedGit")
                .field("repository_url", &"[REDACTED]")
                .field("branch", branch)
                .field("vault_subdirectory", vault_subdirectory)
                .field("mode", mode)
                .field("poll_interval_secs", poll_interval_secs)
                .finish(),
        }
    }
}

impl VaultSource {
    /// The configured poll interval for a managed-Git source, or `None` for
    /// any other source kind. `Local` never polls; `ExistingGit`
    /// `PullOnly`/`TwoWay` Vaults get only the one-shot Git turn activation
    /// already requests and are never registered with `ManagedGitScheduler`
    /// (see `git::managed_task`'s module doc), so they carry no interval.
    pub fn managed_git_poll_interval(&self) -> Option<Duration> {
        match self {
            Self::ManagedGit {
                poll_interval_secs, ..
            } => Some(Duration::from_secs(*poll_interval_secs)),
            Self::Local { .. } | Self::ExistingGit { .. } => None,
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

/// Username handed to `git2::Cred::userpass_plaintext` when a caller supplies
/// a token without a username. `git2` requires a non-empty username argument,
/// but HTTPS token authentication providers (GitHub, GitLab, Gitea, and
/// others) accept any non-empty value paired with the token as the password,
/// so the management UI and MCP callers no longer need to invent one.
pub const HTTPS_CREDENTIALS_USERNAME_PLACEHOLDER: &str = "x-access-token";

#[derive(Clone, PartialEq, Eq)]
pub struct HttpsCredentials {
    pub username: String,
    pub token: String,
}

/// A Vault's own commit author identity, overriding the server-wide
/// `HATCHDOOR_GIT_AUTHOR_NAME`/`HATCHDOOR_GIT_AUTHOR_EMAIL` settings for every
/// commit the managed sync makes on its behalf. Not a secret: unlike
/// [`HttpsCredentials`], it is safe to read back and log.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VaultCommitIdentity {
    pub name: String,
    pub email: String,
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

impl From<StoredHttpsCredentials> for HttpsCredentials {
    fn from(credentials: StoredHttpsCredentials) -> Self {
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
    /// Absent by default: the server-wide `HATCHDOOR_ARCHIVE_PREFIX` applies.
    pub archive_folder: Option<String>,
    /// Absent by default: the server-wide author identity applies.
    pub commit_identity: Option<VaultCommitIdentity>,
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
    /// Absent means the server-wide `HATCHDOOR_ARCHIVE_PREFIX` applies.
    pub archive_folder: Option<String>,
    /// Absent means the server-wide author identity applies.
    pub commit_identity: Option<VaultCommitIdentity>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VaultDefinition {
    vault_id: VaultId,
    name: String,
    enabled: bool,
    source: VaultSource,
    exclude_patterns: Vec<String>,
    credential_configured: bool,
    archive_folder: Option<String>,
    commit_identity: Option<VaultCommitIdentity>,
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

    /// This Vault's own archive folder, normalized with a single trailing
    /// slash (e.g. `"Archive/"`). Absent means the server-wide
    /// `HATCHDOOR_ARCHIVE_PREFIX` applies.
    pub fn archive_folder(&self) -> Option<&str> {
        self.archive_folder.as_deref()
    }

    /// This Vault's own commit author identity. Absent means the server-wide
    /// `HATCHDOOR_GIT_AUTHOR_NAME`/`HATCHDOOR_GIT_AUTHOR_EMAIL` settings apply.
    pub fn commit_identity(&self) -> Option<&VaultCommitIdentity> {
        self.commit_identity.as_ref()
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
    /// Defaulted on deserialization so an on-disk registry written before
    /// this field existed keeps loading under the same
    /// `REGISTRY_SCHEMA_VERSION`, mirroring `VaultSource::ManagedGit`'s
    /// `poll_interval_secs` precedent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    archive_folder: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    commit_identity: Option<VaultCommitIdentity>,
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
            archive_folder: self.archive_folder.clone(),
            commit_identity: self.commit_identity.clone(),
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
            archive_folder: None,
            commit_identity: None,
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

    /// Plaintext credentials for one Vault, for internal Git authentication
    /// only. An absent Vault is indistinguishable from one with no configured
    /// credentials. Never expose this beyond a trusted internal caller: the
    /// redacted `VaultDefinition` projection is the only credential-adjacent
    /// view external surfaces may see.
    pub(crate) fn https_credentials(&self, vault_id: VaultId) -> Option<HttpsCredentials> {
        self.vaults
            .get(&vault_id)?
            .https_credentials
            .clone()
            .map(HttpsCredentials::from)
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
    InvalidArchiveFolder,
    InvalidCommitIdentity,
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
            Self::InvalidArchiveFolder => {
                formatter.write_str("Vault archive folder must be a non-empty relative path")
            }
            Self::InvalidCommitIdentity => formatter.write_str(
                "Vault commit identity must supply a non-empty name and email together",
            ),
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

    /// Persist an intentionally empty registry so legacy import stays disabled.
    pub fn initialize_empty(
        &self,
        expected_revision: u64,
    ) -> Result<VaultRegistrySnapshot, VaultRegistryError> {
        self.commit(expected_revision, std::iter::empty())
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
        let archive_folder = normalize_archive_folder(definition.archive_folder)?;
        let commit_identity = normalize_commit_identity(definition.commit_identity)?;
        vaults.insert(
            vault_id,
            VaultRecord {
                name,
                enabled: definition.enabled,
                source,
                exclude_patterns,
                https_credentials,
                archive_folder,
                commit_identity,
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
                archive_folder: normalize_archive_folder(edit.archive_folder)?,
                commit_identity: normalize_commit_identity(edit.commit_identity)?,
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
            if let Some(folder) = &record.archive_folder
                && (folder.trim_matches('/').is_empty()
                    || !folder.ends_with('/')
                    || folder.chars().any(char::is_control))
            {
                return Err("Vault definition contains an invalid archive folder");
            }
            if let Some(identity) = &record.commit_identity
                && (identity.name.trim().is_empty()
                    || identity.email.trim().is_empty()
                    || identity.name.trim() != identity.name
                    || identity.email.trim() != identity.email
                    || identity.name.chars().any(char::is_control)
                    || identity.email.chars().any(char::is_control))
            {
                return Err("Vault definition contains an invalid commit identity");
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

    /// Plaintext credentials for one Vault, for internal Git authentication
    /// only. Returns `Ok(None)` both when the Vault has no configured
    /// credentials and when it does not exist, so a caller cannot use this to
    /// probe Vault existence. A registry in recovery has no readable Vaults.
    pub(crate) fn https_credentials(
        &self,
        vault_id: VaultId,
    ) -> Result<Option<HttpsCredentials>, VaultRegistryError> {
        Ok(match self.load()? {
            VaultRegistryState::Ready(snapshot) => snapshot.https_credentials(vault_id),
            VaultRegistryState::Recovery(_) => None,
        })
    }

    /// Resolve the local Markdown root owned by a persisted Vault definition.
    ///
    /// Managed checkouts are placed relative to this store's instance-state
    /// directory, so runtime consumers must use this method rather than
    /// reconstructing that persistent layout independently.
    pub fn vault_path(&self, definition: &VaultDefinition) -> PathBuf {
        self.source_vault_path(definition.vault_id(), definition.source())
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
            poll_interval_secs,
        } => Ok(VaultSource::ManagedGit {
            repository_url,
            branch,
            vault_subdirectory,
            mode,
            poll_interval_secs,
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
            poll_interval_secs,
        } => {
            if mode == VaultGitMode::LocalHistory {
                return Err(invalid_source(
                    "managed Git requires pull_only or two_way mode",
                ));
            }
            if poll_interval_secs < MIN_MANAGED_GIT_POLL_INTERVAL_SECS {
                return Err(invalid_source(&format!(
                    "managed Git poll interval must be at least {MIN_MANAGED_GIT_POLL_INTERVAL_SECS} seconds"
                )));
            }
            Ok(VaultSource::ManagedGit {
                repository_url: normalize_https_repository_url(repository_url)?,
                branch: normalize_optional_branch(branch)?,
                vault_subdirectory: vault_subdirectory
                    .map(normalize_relative_subdirectory)
                    .transpose()?,
                mode,
                poll_interval_secs,
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
    let token = credentials.token.trim().to_string();
    if token.is_empty() {
        return Err(VaultRegistryError::InvalidDefinition(
            VaultDefinitionError::InvalidSource(
                "HTTPS credential token must be non-empty".to_string(),
            ),
        ));
    }
    let username = credentials.username.trim().to_string();
    let username = if username.is_empty() {
        HTTPS_CREDENTIALS_USERNAME_PLACEHOLDER.to_string()
    } else {
        username
    };
    Ok(Some(StoredHttpsCredentials { username, token }))
}

/// Normalize a per-Vault archive folder to a single-trailing-slash form
/// (e.g. `"Archive/"`), matching the shape the server-wide
/// `HATCHDOOR_ARCHIVE_PREFIX` setting already uses — so either value can be
/// used interchangeably by every archive-prefix call site.
fn normalize_archive_folder(folder: Option<String>) -> Result<Option<String>, VaultRegistryError> {
    let Some(folder) = folder else {
        return Ok(None);
    };
    let trimmed = folder.trim().trim_matches('/');
    if trimmed.is_empty() || trimmed.chars().any(char::is_control) {
        return Err(VaultRegistryError::InvalidDefinition(
            VaultDefinitionError::InvalidArchiveFolder,
        ));
    }
    Ok(Some(format!("{trimmed}/")))
}

fn normalize_commit_identity(
    identity: Option<VaultCommitIdentity>,
) -> Result<Option<VaultCommitIdentity>, VaultRegistryError> {
    let Some(identity) = identity else {
        return Ok(None);
    };
    let name = identity.name.trim().to_string();
    let email = identity.email.trim().to_string();
    if name.is_empty()
        || email.is_empty()
        || name.chars().any(char::is_control)
        || email.chars().any(char::is_control)
    {
        return Err(VaultRegistryError::InvalidDefinition(
            VaultDefinitionError::InvalidCommitIdentity,
        ));
    }
    Ok(Some(VaultCommitIdentity { name, email }))
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

/// Shared managed-Git input validation. Callers must never accept a broader
/// URL syntax than the persisted Vault source contract.
///
/// Deliberately has no `#[cfg(test)]` local-path allowance, unlike
/// `git::managed_checkout::is_safe_managed_repository_url` and
/// `git::managed_sync::safe_managed_remote_url`, which each carry one so
/// their own tests can drive real `git2` clone/fetch/push against a local
/// bare repository. This validator gates the persisted registry itself — a
/// non-`https://` `ManagedGit` source is corruption at both write time
/// (`add`/`edit`) and read time (`load`), with no exception. A test that
/// needs a `VaultDefinition`/`VaultControlBlock` wired to a real local `git2`
/// fixture cannot get one through `VaultRegistryStore` and should not try;
/// either exercise the git2 layer directly (as `managed_checkout.rs`/
/// `managed_sync.rs`/`managed_task.rs` do), or inject a fake turn executor at
/// the `vault_runtime::dispatch_managed_git_turn_with` seam to drive a
/// fabricated result through the full async/status-publishing path instead.
pub(crate) fn is_safe_https_repository_url(repository_url: &str) -> bool {
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
mod tests;
