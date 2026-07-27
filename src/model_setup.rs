//! Persistent, local-only selection and acknowledgement state for Hatchdoor's
//! startup embedding model. Model files live under the same directory, so
//! moving the bind mount preserves both the weights and their terms receipt.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use hf_hub::api::Progress;
use hf_hub::api::sync::ApiBuilder;
use hf_hub::{Cache, Repo, RepoType};
use serde::{Deserialize, Serialize};

pub const GEMMA_MODEL_ID: &str = "EmbeddingGemma 300M Q4";
pub const NOMIC_MODEL_ID: &str = "Nomic Embed Text v1.5";
pub const GEMMA_TERMS_URL: &str = "https://ai.google.dev/gemma/terms";
pub const GEMMA_POLICY_URL: &str = "https://ai.google.dev/gemma/prohibited_use_policy";
pub const GEMMA_TERMS_VERSION: &str = "2026-04-01";
pub const GEMMA_REPOSITORY: &str = "onnx-community/embeddinggemma-300m-ONNX";
pub const GEMMA_REVISION: &str = "5090578d9565bb06545b4552f76e6bc2c93e4a66";
/// Enough room for either model plus temporary download files and a fresh
/// index. The cache is persistent, so setup must fail early rather than leave
/// a half-written model on a full volume.
pub const MIN_SETUP_FREE_BYTES: u64 = 512 * 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SelectedModel {
    TermsRequired,
    Gemma,
    Nomic,
}

impl SelectedModel {
    pub fn id(self) -> Option<&'static str> {
        match self {
            Self::TermsRequired => None,
            Self::Gemma => Some(GEMMA_MODEL_ID),
            Self::Nomic => Some(NOMIC_MODEL_ID),
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct SelectionRecord {
    selected: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct AcceptanceRecord {
    terms_version: String,
    terms_url: String,
    prohibited_use_policy_url: String,
    accepted_at: String,
    model_repository: String,
    model_revision: String,
    hatchdoor_version: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct IntegrityRecord {
    artifacts: Vec<IntegrityArtifact>,
}

#[derive(Debug, Serialize, Deserialize)]
struct IntegrityArtifact {
    path: String,
    blake3: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CacheIntegrity {
    Missing,
    Valid,
    Invalid,
}

#[derive(Debug, Clone)]
pub struct ModelSetup {
    models_dir: PathBuf,
}

impl ModelSetup {
    pub fn new(models_dir: PathBuf) -> Self {
        Self { models_dir }
    }

    pub fn default_models_dir() -> PathBuf {
        if cfg!(target_os = "linux") && Path::new("/models").is_dir() {
            PathBuf::from("/models")
        } else {
            PathBuf::from("models")
        }
    }

    pub fn models_dir(&self) -> &Path {
        &self.models_dir
    }

    pub fn model_cache_dir(&self, selected: SelectedModel) -> PathBuf {
        match selected {
            SelectedModel::Gemma => self
                .models_dir
                .join("embeddinggemma-300m-q4")
                .join(GEMMA_REVISION),
            SelectedModel::Nomic => self.models_dir.join("nomic-embed-text-v1.5"),
            SelectedModel::TermsRequired => self.models_dir.join("pending"),
        }
    }

    pub fn selected(&self) -> Result<SelectedModel, String> {
        let selection_path = self.models_dir.join("selection.json");
        let Ok(bytes) = std::fs::read(selection_path) else {
            return Ok(SelectedModel::TermsRequired);
        };
        let selection: SelectionRecord = serde_json::from_slice(&bytes)
            .map_err(|error| format!("invalid local model selection: {error}"))?;
        match selection.selected.as_str() {
            "gemma" if self.acceptance_is_current()? => Ok(SelectedModel::Gemma),
            "nomic" => Ok(SelectedModel::Nomic),
            _ => Ok(SelectedModel::TermsRequired),
        }
    }

    pub fn accept_gemma(&self) -> Result<(), String> {
        let dir = self.model_cache_dir(SelectedModel::Gemma);
        std::fs::create_dir_all(&dir)
            .map_err(|error| format!("create model directory: {error}"))?;
        let receipt = AcceptanceRecord {
            terms_version: GEMMA_TERMS_VERSION.to_string(),
            terms_url: GEMMA_TERMS_URL.to_string(),
            prohibited_use_policy_url: GEMMA_POLICY_URL.to_string(),
            accepted_at: chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
            model_repository: GEMMA_REPOSITORY.to_string(),
            model_revision: GEMMA_REVISION.to_string(),
            hatchdoor_version: env!("CARGO_PKG_VERSION").to_string(),
        };
        write_json_atomic(&dir.join("acceptance.json"), &receipt)?;
        self.write_selection("gemma")
    }

    pub fn decline_gemma(&self) -> Result<(), String> {
        let gemma_root = self.models_dir.join("embeddinggemma-300m-q4");
        if gemma_root.exists() {
            std::fs::remove_dir_all(&gemma_root)
                .map_err(|error| format!("remove declined Gemma model: {error}"))?;
        }
        self.write_selection("nomic")
    }

    /// Prepare a persistent model directory for an interrupted download to
    /// resume safely. A changed cached artifact is removed before FastEmbed is
    /// allowed to use it; a missing manifest simply means this is the first
    /// download.
    pub fn prepare_download(&self, selected: SelectedModel) -> Result<(), String> {
        let dir = self.model_cache_dir(selected);
        std::fs::create_dir_all(&dir)
            .map_err(|error| format!("create model directory: {error}"))?;
        let available = available_bytes(&dir)?;
        if available < MIN_SETUP_FREE_BYTES {
            return Err(format!(
                "not enough free space for model setup: {} MB available, at least {} MB required",
                available / (1024 * 1024),
                MIN_SETUP_FREE_BYTES / (1024 * 1024)
            ));
        }
        if self.verify_integrity(selected)? == CacheIntegrity::Invalid {
            self.remove_downloaded_artifacts(selected)?;
        }
        Ok(())
    }

    /// Download every artifact FastEmbed needs through a fixed Gemma revision,
    /// then point FastEmbed's normal `main` cache lookup at that immutable
    /// snapshot. FastEmbed itself does not expose a revision option.
    pub fn fetch_gemma_at_pinned_revision(
        &self,
        report: Arc<dyn Fn(u64, u64) + Send + Sync>,
    ) -> Result<(), String> {
        let cache_dir = self.model_cache_dir(SelectedModel::Gemma);
        let api = ApiBuilder::new()
            .with_cache_dir(cache_dir.clone())
            .with_progress(false)
            .build()
            .map_err(|error| format!("create Hugging Face download client: {error}"))?;
        let pinned_repo = Repo::with_revision(
            GEMMA_REPOSITORY.to_string(),
            RepoType::Model,
            GEMMA_REVISION.to_string(),
        );
        let cached = Cache::new(cache_dir.clone()).repo(pinned_repo.clone());
        let pinned = api.repo(pinned_repo);
        for file in [
            "onnx/model_q4.onnx",
            "onnx/model_q4.onnx_data",
            "config.json",
            "special_tokens_map.json",
            "tokenizer.json",
            "tokenizer_config.json",
        ] {
            if cached.get(file).is_none() {
                pinned
                    .download_with_progress(file, ModelDownloadProgress::new(report.clone()))
                    .map_err(|error| format!("download Gemma artifact {file}: {error}"))?;
            }
        }
        // FastEmbed uses Repo::model (revision "main"). Its cache supports a
        // local ref, so write `main -> <pinned commit>` only after every needed
        // artifact landed in the snapshot.
        Cache::new(cache_dir)
            .repo(Repo::model(GEMMA_REPOSITORY.to_string()))
            .create_ref(GEMMA_REVISION)
            .map_err(|error| format!("activate pinned Gemma snapshot: {error}"))
    }

    /// Record all downloaded regular files after a successful load. The next
    /// startup verifies these BLAKE3 digests before reusing the local cache.
    pub fn record_integrity(&self, selected: SelectedModel) -> Result<(), String> {
        let dir = self.model_cache_dir(selected);
        let mut artifacts = Vec::new();
        for entry in walkdir::WalkDir::new(&dir) {
            let entry = entry.map_err(|error| format!("walk model cache: {error}"))?;
            if !entry.file_type().is_file() || is_local_metadata(entry.path()) {
                continue;
            }
            let relative = entry
                .path()
                .strip_prefix(&dir)
                .map_err(|error| format!("model cache relative path: {error}"))?
                .to_string_lossy()
                .into_owned();
            artifacts.push(IntegrityArtifact {
                path: relative,
                blake3: file_blake3(entry.path())?,
            });
        }
        artifacts.sort_by(|left, right| left.path.cmp(&right.path));
        if artifacts.is_empty() {
            return Err("model download completed without cache artifacts".to_string());
        }
        write_json_atomic(&dir.join("integrity.json"), &IntegrityRecord { artifacts })
    }

    pub fn verify_integrity(&self, selected: SelectedModel) -> Result<CacheIntegrity, String> {
        let dir = self.model_cache_dir(selected);
        let path = dir.join("integrity.json");
        let Ok(bytes) = std::fs::read(path) else {
            return Ok(CacheIntegrity::Missing);
        };
        let record: IntegrityRecord = serde_json::from_slice(&bytes)
            .map_err(|error| format!("invalid model integrity record: {error}"))?;
        if record.artifacts.is_empty() {
            return Ok(CacheIntegrity::Invalid);
        }
        for artifact in record.artifacts {
            let path = dir.join(&artifact.path);
            if !path.is_file() || file_blake3(&path)? != artifact.blake3 {
                return Ok(CacheIntegrity::Invalid);
            }
        }
        Ok(CacheIntegrity::Valid)
    }

    /// Discard downloaded artifacts while retaining a Gemma acceptance receipt.
    /// Used after an unrecoverable first attempt before one clean retry.
    pub fn reset_download_cache(&self, selected: SelectedModel) -> Result<(), String> {
        self.remove_downloaded_artifacts(selected)
    }

    fn remove_downloaded_artifacts(&self, selected: SelectedModel) -> Result<(), String> {
        let dir = self.model_cache_dir(selected);
        for entry in
            std::fs::read_dir(&dir).map_err(|error| format!("read corrupt model cache: {error}"))?
        {
            let entry =
                entry.map_err(|error| format!("read corrupt model cache entry: {error}"))?;
            let path = entry.path();
            if is_local_metadata(&path) {
                continue;
            }
            if path.is_dir() {
                std::fs::remove_dir_all(&path)
                    .map_err(|error| format!("remove corrupt model cache: {error}"))?;
            } else {
                std::fs::remove_file(&path)
                    .map_err(|error| format!("remove corrupt model artifact: {error}"))?;
            }
        }
        Ok(())
    }

    fn acceptance_is_current(&self) -> Result<bool, String> {
        let path = self
            .model_cache_dir(SelectedModel::Gemma)
            .join("acceptance.json");
        let Ok(bytes) = std::fs::read(path) else {
            return Ok(false);
        };
        let receipt: AcceptanceRecord = serde_json::from_slice(&bytes)
            .map_err(|error| format!("invalid Gemma acceptance receipt: {error}"))?;
        Ok(receipt.terms_version == GEMMA_TERMS_VERSION
            && receipt.model_repository == GEMMA_REPOSITORY
            && receipt.model_revision == GEMMA_REVISION)
    }

    fn write_selection(&self, selected: &str) -> Result<(), String> {
        std::fs::create_dir_all(&self.models_dir)
            .map_err(|error| format!("create models directory: {error}"))?;
        write_json_atomic(
            &self.models_dir.join("selection.json"),
            &SelectionRecord {
                selected: selected.to_string(),
            },
        )
    }
}

fn is_local_metadata(path: &Path) -> bool {
    matches!(
        path.file_name().and_then(|name| name.to_str()),
        Some("acceptance.json" | "integrity.json")
    )
}

fn file_blake3(path: &Path) -> Result<String, String> {
    let mut file = std::fs::File::open(path)
        .map_err(|error| format!("open model artifact {}: {error}", path.display()))?;
    let mut hasher = blake3::Hasher::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|error| format!("read model artifact {}: {error}", path.display()))?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(hasher.finalize().to_hex().to_string())
}

fn available_bytes(path: &Path) -> Result<u64, String> {
    #[cfg(unix)]
    {
        use std::ffi::CString;
        let path = CString::new(path.as_os_str().as_encoded_bytes())
            .map_err(|_| "model path contains an interior NUL byte".to_string())?;
        let mut stat = std::mem::MaybeUninit::<libc::statvfs>::uninit();
        // SAFETY: `path` is NUL-terminated and `stat` is valid writable storage.
        let result = unsafe { libc::statvfs(path.as_ptr(), stat.as_mut_ptr()) };
        if result != 0 {
            return Err(format!(
                "check free space for model directory: {}",
                std::io::Error::last_os_error()
            ));
        }
        // SAFETY: successful statvfs initializes the output structure.
        let stat = unsafe { stat.assume_init() };
        Ok(stat.f_bavail.saturating_mul(stat.f_frsize))
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        Ok(u64::MAX)
    }
}

struct ModelDownloadProgress {
    report: Arc<dyn Fn(u64, u64) + Send + Sync>,
    downloaded: u64,
    total: u64,
}

impl ModelDownloadProgress {
    fn new(report: Arc<dyn Fn(u64, u64) + Send + Sync>) -> Self {
        Self {
            report,
            downloaded: 0,
            total: 0,
        }
    }
}

impl Progress for ModelDownloadProgress {
    fn init(&mut self, size: usize, _filename: &str) {
        self.total = size as u64;
        (self.report)(0, self.total);
    }

    fn update(&mut self, size: usize) {
        self.downloaded = self.downloaded.saturating_add(size as u64);
        (self.report)(self.downloaded, self.total);
    }

    fn finish(&mut self) {
        (self.report)(self.total, self.total);
    }
}

fn write_json_atomic(path: &Path, value: &impl Serialize) -> Result<(), String> {
    let bytes =
        serde_json::to_vec_pretty(value).map_err(|error| format!("encode json: {error}"))?;
    let temporary = path.with_extension("tmp");
    std::fs::write(&temporary, bytes)
        .map_err(|error| format!("write {}: {error}", temporary.display()))?;
    std::fs::rename(&temporary, path)
        .map_err(|error| format!("replace {}: {error}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fresh_model_directory_requires_gemma_terms() {
        let temp = tempfile::tempdir().expect("tempdir");
        assert_eq!(
            ModelSetup::new(temp.path().join("models"))
                .selected()
                .unwrap(),
            SelectedModel::TermsRequired
        );
    }

    #[test]
    fn accepting_gemma_persists_a_local_versioned_receipt() {
        let temp = tempfile::tempdir().expect("tempdir");
        let setup = ModelSetup::new(temp.path().join("models"));
        setup.accept_gemma().expect("accept");
        assert_eq!(setup.selected().unwrap(), SelectedModel::Gemma);
        let receipt = setup
            .model_cache_dir(SelectedModel::Gemma)
            .join("acceptance.json");
        let text = std::fs::read_to_string(receipt).expect("receipt");
        assert!(text.contains(GEMMA_TERMS_VERSION));
        assert!(text.contains(GEMMA_REVISION));
        assert!(!text.contains("vault"));
    }

    #[test]
    fn declining_gemma_removes_its_files_and_selects_nomic() {
        let temp = tempfile::tempdir().expect("tempdir");
        let setup = ModelSetup::new(temp.path().join("models"));
        setup.accept_gemma().expect("accept");
        let gemma_root = setup.models_dir().join("embeddinggemma-300m-q4");
        std::fs::write(gemma_root.join("partial.onnx"), "partial").expect("partial file");
        setup.decline_gemma().expect("decline");
        assert!(!gemma_root.exists());
        assert_eq!(setup.selected().unwrap(), SelectedModel::Nomic);
    }

    #[test]
    fn integrity_manifest_detects_a_changed_cached_artifact() {
        let temp = tempfile::tempdir().expect("tempdir");
        let setup = ModelSetup::new(temp.path().join("models"));
        let dir = setup.model_cache_dir(SelectedModel::Nomic);
        std::fs::create_dir_all(&dir).expect("model dir");
        let artifact = dir.join("model.onnx");
        std::fs::write(&artifact, "original model bytes").expect("artifact");
        setup
            .record_integrity(SelectedModel::Nomic)
            .expect("integrity record");
        assert_eq!(
            setup.verify_integrity(SelectedModel::Nomic).unwrap(),
            CacheIntegrity::Valid
        );

        std::fs::write(artifact, "corrupt bytes").expect("corrupt artifact");
        assert_eq!(
            setup.verify_integrity(SelectedModel::Nomic).unwrap(),
            CacheIntegrity::Invalid
        );
    }
}
