//! Persistent, local-only selection and acknowledgement state for Hatchdoor's
//! startup embedding model. Model files live under the same directory, so
//! moving the bind mount preserves both the weights and their terms receipt.

use std::io::{BufRead, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::time::{Duration, Instant};

use hf_hub::api::Progress;
use hf_hub::api::sync::{Api, ApiBuilder};
use hf_hub::{Cache, Repo, RepoType};
use serde::{Deserialize, Serialize};

pub const GEMMA_MODEL_ID: &str = "EmbeddingGemma 300M Q4";
pub const NOMIC_MODEL_ID: &str = "Nomic Embed Text v1.5";
pub const GEMMA_TERMS_URL: &str = "https://ai.google.dev/gemma/terms";
pub const GEMMA_POLICY_URL: &str = "https://ai.google.dev/gemma/prohibited_use_policy";
pub const GEMMA_TERMS_VERSION: &str = "2026-04-01";
pub const GEMMA_REPOSITORY: &str = "onnx-community/embeddinggemma-300m-ONNX";
pub const GEMMA_REVISION: &str = "5090578d9565bb06545b4552f76e6bc2c93e4a66";
pub const NOMIC_REPOSITORY: &str = "nomic-ai/nomic-embed-text-v1.5";
/// The Nomic commit every install loads (#643). It was `main` from April
/// 2026, so a cache FastEmbed filled from `main` since then already holds it.
pub const NOMIC_REVISION: &str = "e9b6763023c676ca8431644204f50c2b100d9aab";

/// One file a model's load reads, with the SHA-256 of its bytes at the pinned
/// revision. The large files' digests are Hugging Face's own LFS SHA-256.
/// For the small files Hugging Face publishes only a Git blob ID, so their
/// SHA-256 was computed from copies whose Git blob ID matched it.
#[derive(Clone, Copy, Debug)]
struct ModelFile {
    path: &'static str,
    sha256: &'static str,
}

/// A model as Hatchdoor loads it: a repository at one commit, and the files
/// of that commit its load reads.
#[derive(Clone, Copy, Debug)]
struct PinnedModel {
    repository: &'static str,
    revision: &'static str,
    files: &'static [ModelFile],
}

/// What FastEmbed reads for Nomic v1.5 (`EmbeddingModel::NomicEmbedTextV15`
/// in fastembed 5.17): the ONNX graph and the four tokenizer files.
const NOMIC: PinnedModel = PinnedModel {
    repository: NOMIC_REPOSITORY,
    revision: NOMIC_REVISION,
    files: &[
        ModelFile {
            path: "onnx/model.onnx",
            sha256: "147d5aa88c2101237358e17796cf3a227cead1ec304ec34b465bb08e9d952965",
        },
        ModelFile {
            path: "config.json",
            sha256: "9ab00bd92cee80a569f708140b7b6c1661a65891ff3765b1519e181ba2f2c92b",
        },
        ModelFile {
            path: "special_tokens_map.json",
            sha256: "5d5b662e421ea9fac075174bb0688ee0d9431699900b90662acd44b2a350503a",
        },
        ModelFile {
            path: "tokenizer.json",
            sha256: "d241a60d5e8f04cc1b2b3e9ef7a4921b27bf526d9f6050ab90f9267a1f9e5c66",
        },
        ModelFile {
            path: "tokenizer_config.json",
            sha256: "d7e0000bcc80134debd2222220427e6bf5fa20a669f40a0d0d1409cc18e0a9bc",
        },
    ],
};

/// What `FastembedEmbedder::embedding_gemma_300m_q4_from` reads.
const GEMMA: PinnedModel = PinnedModel {
    repository: GEMMA_REPOSITORY,
    revision: GEMMA_REVISION,
    files: &[
        ModelFile {
            path: "onnx/model_q4.onnx",
            sha256: "ad1dfee81a70f7944b9b9d1cc6e48075b832881cf33fab2f2b248be78f3f0043",
        },
        ModelFile {
            path: "onnx/model_q4.onnx_data",
            sha256: "599962c3143b040de2dd05e5975be3e9091dd067cacc6a8f7186e3203bab9e02",
        },
        ModelFile {
            path: "config.json",
            sha256: "6e1f06404b7163e0325ed2ea3e6781cde50f4a50b31780a95ad0d30e8404d77b",
        },
        ModelFile {
            path: "special_tokens_map.json",
            sha256: "2f7b0adf4fb469770bb1490e3e35df87b1dc578246c5e7e6fc76ecf33213a397",
        },
        ModelFile {
            path: "tokenizer.json",
            sha256: "4dda02faaf32bc91031dc8c88457ac272b00c1016cc679757d1c441b248b9c47",
        },
        ModelFile {
            path: "tokenizer_config.json",
            sha256: "3ca953eea6c3c9fcda9cf3df22949ff18b216f7c74bd6459230f3f1013953f3a",
        },
    ],
};

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

    /// The model this install chose. A missing or unreadable choice, or a
    /// Gemma choice without a current terms receipt, asks for the choice
    /// again rather than stopping Hatchdoor (#643).
    pub fn selected(&self) -> SelectedModel {
        let Some(selection) = read_record::<SelectionRecord>(
            &self.models_dir.join("selection.json"),
            "model selection",
        ) else {
            return SelectedModel::TermsRequired;
        };
        match selection.selected.as_str() {
            "gemma" if self.acceptance_is_current() => SelectedModel::Gemma,
            "nomic" => SelectedModel::Nomic,
            _ => SelectedModel::TermsRequired,
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

    /// Check the cached model before setup uses it. A cache whose files all
    /// match their known digests is returned `Valid` and needs no download. A
    /// changed file is removed with the rest of the cache, so it is fetched
    /// again before FastEmbed can use it, and the answer is then `Missing`.
    /// Only a cache that needs a download must find
    /// [`MIN_SETUP_FREE_BYTES`] free (#643).
    ///
    /// A cache that is not `Valid` is about to change, so this records that
    /// first ([`Self::files_changed_since_index`]).
    pub fn prepare_download(&self, selected: SelectedModel) -> Result<CacheIntegrity, String> {
        let dir = self.model_cache_dir(selected);
        let integrity = prepare_cache(&dir, pinned(selected)?, MIN_SETUP_FREE_BYTES)?;
        if integrity != CacheIntegrity::Valid {
            mark_files_changed(&dir)?;
        }
        Ok(integrity)
    }

    /// Whether the selected model's files were fetched, replaced or
    /// re-pointed since a search index last accepted them. Kept on disk, so
    /// a start that changed the files and stopped before the index check
    /// still counts as a change at the next start (#693).
    pub fn files_changed_since_index(&self, selected: SelectedModel) -> bool {
        self.model_cache_dir(selected)
            .join(FILES_CHANGED_RECORD)
            .exists()
    }

    /// Record that the search index has checked itself against the selected
    /// model's current files.
    pub fn files_reached_index(&self, selected: SelectedModel) -> Result<(), String> {
        match std::fs::remove_file(self.model_cache_dir(selected).join(FILES_CHANGED_RECORD)) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(format!("clear the model files change record: {error}")),
        }
    }

    /// The folder holding the selected model's files at its pinned revision,
    /// the paths its load opens.
    pub fn snapshot_dir(&self, selected: SelectedModel) -> Result<PathBuf, String> {
        Ok(snapshot_dir(
            &self.model_cache_dir(selected),
            pinned(selected)?,
        ))
    }

    /// Check every file the selected model's load reads against the digest
    /// Hatchdoor ships for it, through the paths the load opens.
    fn verify_cache(&self, selected: SelectedModel) -> Result<CacheIntegrity, String> {
        let model = pinned(selected)?;
        verify_snapshot(&snapshot_dir(&self.model_cache_dir(selected), model), model)
    }

    /// Refuse a download that did not leave exactly the pinned files, before
    /// anything loads them (#643).
    pub fn verify_download(&self, selected: SelectedModel) -> Result<(), String> {
        let model = pinned(selected)?;
        match self.verify_cache(selected)? {
            CacheIntegrity::Valid => Ok(()),
            CacheIntegrity::Missing => Err(format!(
                "model files for {} are missing after the download",
                model.repository
            )),
            CacheIntegrity::Invalid => Err(format!(
                "model files for {} do not match their known digests",
                model.repository
            )),
        }
    }

    /// Point the cache's `main` at the pinned commit. FastEmbed's Nomic load
    /// takes no revision and reads `main`, so this is what keeps it on the
    /// files [`Self::verify_cache`] checked. A ref already right is left
    /// alone, so a valid cache on a full volume is not written to, and a
    /// failed write cannot send setup into resetting a good cache. Moving it
    /// counts as a change of files: a Nomic load before #643 read whatever
    /// commit `main` named (#693).
    pub fn point_main_at_pinned_revision(&self, selected: SelectedModel) -> Result<(), String> {
        let model = pinned(selected)?;
        let main = self
            .model_cache_dir(selected)
            .join(Repo::model(model.repository.to_string()).folder_name())
            .join("refs")
            .join("main");
        if std::fs::read_to_string(&main).is_ok_and(|current| current.trim() == model.revision) {
            return Ok(());
        }
        mark_files_changed(&self.model_cache_dir(selected))?;
        Cache::new(self.model_cache_dir(selected))
            .repo(Repo::model(model.repository.to_string()))
            .create_ref(model.revision)
            .map_err(|error| format!("point the model cache at the pinned revision: {error}"))
    }

    /// Download every file the selected model's load reads, so the load
    /// itself never touches the network. Runs in the child process
    /// [`run_supervised_fetch`] watches, because nothing in `hf-hub` bounds a
    /// request that stops answering (#642).
    pub fn fetch_model(
        &self,
        selected: SelectedModel,
        report: Arc<dyn Fn(u64, u64) + Send + Sync>,
    ) -> Result<(), String> {
        let source = DownloadSource {
            endpoint: std::env::var("HF_ENDPOINT")
                .unwrap_or_else(|_| "https://huggingface.co".to_string()),
        };
        self.fetch_model_from(selected, &source, report)
    }

    fn fetch_model_from(
        &self,
        selected: SelectedModel,
        source: &DownloadSource,
        report: Arc<dyn Fn(u64, u64) + Send + Sync>,
    ) -> Result<(), String> {
        fetch_pinned(
            &self.model_cache_dir(selected),
            pinned(selected)?,
            source,
            report,
        )
    }

    /// The command that runs [`Self::fetch_model`] in a child copy of this
    /// executable, for [`run_supervised_fetch`] to watch.
    pub fn fetch_command(&self, selected: SelectedModel) -> Result<Command, String> {
        let model = match selected {
            SelectedModel::Gemma => "gemma",
            SelectedModel::Nomic => "nomic",
            SelectedModel::TermsRequired => {
                return Err("model terms have not been accepted".to_string());
            }
        };
        let program = std::env::current_exe()
            .map_err(|error| format!("locate the Hatchdoor executable: {error}"))?;
        let mut command = Command::new(program);
        command
            .arg(FETCH_MODEL_FLAG)
            .arg(model)
            .arg(&self.models_dir);
        #[cfg(target_os = "linux")]
        {
            use std::os::unix::process::CommandExt;
            // SAFETY: prctl, getppid and _exit are async-signal-safe and
            // touch no memory of the parent. The child dies with the thread that started it, so
            // a Hatchdoor that exits mid-download leaves no download behind.
            // That thread is the setup task's blocking thread, which waits
            // for the child for as long as it runs.
            // A parent that died before prctl ran is caught by comparing
            // the parent's process ID afterwards.
            let parent = std::process::id();
            unsafe {
                command.pre_exec(move || {
                    libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL);
                    if libc::getppid() as u32 != parent {
                        libc::_exit(1);
                    }
                    Ok(())
                });
            }
        }
        Ok(command)
    }

    /// Discard downloaded artifacts while retaining a Gemma acceptance receipt.
    /// Used after an unrecoverable first attempt before one clean retry.
    pub fn reset_download_cache(&self, selected: SelectedModel) -> Result<(), String> {
        let dir = self.model_cache_dir(selected);
        mark_files_changed(&dir)?;
        remove_downloaded_artifacts(&dir)
    }

    fn acceptance_is_current(&self) -> bool {
        let path = self
            .model_cache_dir(SelectedModel::Gemma)
            .join("acceptance.json");
        read_record::<AcceptanceRecord>(&path, "Gemma terms receipt").is_some_and(|receipt| {
            receipt.terms_version == GEMMA_TERMS_VERSION
                && receipt.model_repository == GEMMA_REPOSITORY
                && receipt.model_revision == GEMMA_REVISION
        })
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

/// Where model files come from: `HF_ENDPOINT`. They always land in the
/// model's own folder under the models directory, whatever `HF_HOME` says
/// (#643).
struct DownloadSource {
    endpoint: String,
}

impl DownloadSource {
    fn client(&self, cache_dir: PathBuf) -> Result<Api, String> {
        ApiBuilder::new()
            .with_cache_dir(cache_dir)
            .with_endpoint(self.endpoint.clone())
            .with_progress(false)
            .build()
            .map_err(|error| format!("create Hugging Face download client: {error}"))
    }
}

fn pinned(selected: SelectedModel) -> Result<&'static PinnedModel, String> {
    match selected {
        SelectedModel::Gemma => Ok(&GEMMA),
        SelectedModel::Nomic => Ok(&NOMIC),
        SelectedModel::TermsRequired => Err("model terms have not been accepted".to_string()),
    }
}

/// `<cache>/models--<owner>--<name>/snapshots/<revision>`, as `hf-hub` lays
/// a revision out.
fn snapshot_dir(cache_dir: &Path, model: &PinnedModel) -> PathBuf {
    Cache::new(cache_dir.to_path_buf())
        .repo(Repo::with_revision(
            model.repository.to_string(),
            RepoType::Model,
            model.revision.to_string(),
        ))
        .pointer_path(model.revision)
}

/// Download every file of `model` that its snapshot does not hold yet. The
/// presence check reads the snapshot itself rather than a ref, so a cache
/// FastEmbed filled under `main` is reused as it is.
fn fetch_pinned(
    cache_dir: &Path,
    model: &PinnedModel,
    source: &DownloadSource,
    report: Arc<dyn Fn(u64, u64) + Send + Sync>,
) -> Result<(), String> {
    let snapshot = snapshot_dir(cache_dir, model);
    let api = source.client(cache_dir.to_path_buf())?;
    let remote = api.repo(Repo::with_revision(
        model.repository.to_string(),
        RepoType::Model,
        model.revision.to_string(),
    ));
    for file in model.files {
        if !snapshot.join(file.path).exists() {
            remote
                .download_with_progress(file.path, ModelDownloadProgress::new(report.clone()))
                .map_err(|error| {
                    format!(
                        "download {} artifact {}: {error}",
                        model.repository, file.path
                    )
                })?;
        }
    }
    Ok(())
}

/// Hash every file of `model` through `snapshot`, the paths its load opens,
/// following the links `hf-hub` leaves there. A file whose bytes differ, a
/// link that leads nowhere, or a regular file the model does not list makes
/// the cache `Invalid`; a file not downloaded yet makes it `Missing`.
fn verify_snapshot(snapshot: &Path, model: &PinnedModel) -> Result<CacheIntegrity, String> {
    if !snapshot.is_dir() {
        return Ok(CacheIntegrity::Missing);
    }
    let mut missing = false;
    for file in model.files {
        let path = snapshot.join(file.path);
        match std::fs::metadata(&path) {
            Ok(metadata) if metadata.is_file() => {
                if file_sha256(&path)? != file.sha256 {
                    return Ok(CacheIntegrity::Invalid);
                }
            }
            Ok(_) => return Ok(CacheIntegrity::Invalid),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                if std::fs::symlink_metadata(&path).is_ok() {
                    // A link whose target is gone: `hf-hub` cannot replace it.
                    return Ok(CacheIntegrity::Invalid);
                }
                missing = true;
            }
            Err(error) => {
                return Err(format!("read model artifact {}: {error}", path.display()));
            }
        }
    }
    for entry in walkdir::WalkDir::new(snapshot).min_depth(1) {
        let entry = entry.map_err(|error| format!("walk model cache: {error}"))?;
        // Hidden files, such as the `.DS_Store` and `._name` files macOS
        // leaves on a shared folder, are nothing a load opens.
        if !entry.file_type().is_file() || entry.file_name().to_string_lossy().starts_with('.') {
            continue;
        }
        let listed = entry.path().strip_prefix(snapshot).is_ok_and(|relative| {
            model
                .files
                .iter()
                .any(|file| Path::new(file.path) == relative)
        });
        if !listed {
            return Ok(CacheIntegrity::Invalid);
        }
    }
    Ok(if missing {
        CacheIntegrity::Missing
    } else {
        CacheIntegrity::Valid
    })
}

/// Make `dir` ready for setup: drop the record older versions kept, check
/// the cache against `model`, and remove it whole when a file does not match,
/// so it is fetched again before anything loads it. Free space is checked
/// only when a download will follow (#643).
fn prepare_cache(
    dir: &Path,
    model: &PinnedModel,
    min_free_bytes: u64,
) -> Result<CacheIntegrity, String> {
    std::fs::create_dir_all(dir).map_err(|error| format!("create model directory: {error}"))?;
    // The digests a first load recorded, before Hatchdoor shipped its own
    // (#643). Nothing reads it any more.
    let _ = std::fs::remove_file(dir.join(LEGACY_INTEGRITY_RECORD));
    let mut integrity = verify_snapshot(&snapshot_dir(dir, model), model)?;
    if integrity == CacheIntegrity::Invalid {
        remove_downloaded_artifacts(dir)?;
        integrity = CacheIntegrity::Missing;
    }
    if integrity == CacheIntegrity::Valid {
        return Ok(integrity);
    }
    let available = available_bytes(dir)?;
    if available < min_free_bytes {
        return Err(format!(
            "not enough free space for model setup: {} MB available, at least {} MB required",
            available / (1024 * 1024),
            min_free_bytes / (1024 * 1024)
        ));
    }
    Ok(integrity)
}

/// Remove everything downloaded into `dir`, keeping a Gemma terms receipt.
fn remove_downloaded_artifacts(dir: &Path) -> Result<(), String> {
    for entry in
        std::fs::read_dir(dir).map_err(|error| format!("read corrupt model cache: {error}"))?
    {
        let entry = entry.map_err(|error| format!("read corrupt model cache entry: {error}"))?;
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

/// Read one of the small JSON records setup keeps. A record that is missing
/// reads as absent; one that cannot be read or parsed, as after a power cut
/// mid-write, is logged and also reads as absent (#643).
fn read_record<T: serde::de::DeserializeOwned>(path: &Path, what: &str) -> Option<T> {
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return None,
        Err(error) => {
            tracing::warn!("Could not read the {what} at {}: {error}", path.display());
            return None;
        }
    };
    match serde_json::from_slice(&bytes) {
        Ok(record) => Some(record),
        Err(error) => {
            tracing::warn!("Ignoring the damaged {what} at {}: {error}", path.display());
            None
        }
    }
}

/// The command-line flag that runs a model download as a child process.
pub const FETCH_MODEL_FLAG: &str = "--fetch-model";
/// Every line the child writes for its supervisor starts with this, so the
/// supervisor can tell them from anything else on the child's output.
const FETCH_LINE_PREFIX: &str = "hatchdoor-model-fetch";
/// The child reports progress at most this often while data arrives, plus
/// once at the start of every file.
const FETCH_REPORT_EVERY: Duration = Duration::from_millis(250);

/// How long a model download may go without receiving any data before it
/// counts as stalled (#642).
pub const DOWNLOAD_STALL_AFTER: Duration = Duration::from_secs(60);
/// The longest one download attempt may run, however steadily data trickles
/// in. A later attempt resumes the partial files this one leaves.
pub const DOWNLOAD_DEADLINE: Duration = Duration::from_secs(2 * 60 * 60);

#[derive(Clone, Copy, Debug)]
pub struct FetchLimits {
    pub stall_after: Duration,
    pub deadline: Duration,
}

impl Default for FetchLimits {
    fn default() -> Self {
        Self {
            stall_after: DOWNLOAD_STALL_AFTER,
            deadline: DOWNLOAD_DEADLINE,
        }
    }
}

/// Why a supervised download did not complete.
#[derive(Debug, PartialEq, Eq)]
pub enum FetchFailure {
    /// No data arrived for `stall_after`; the child was stopped.
    Stalled(Duration),
    /// The attempt ran past `deadline`; the child was stopped.
    DeadlineExceeded(Duration),
    /// The download itself failed, or the child could not run.
    Failed(String),
}

impl FetchFailure {
    /// The sentence the startup status shows for a stopped download, or
    /// `None` for an ordinary failure, which keeps the generic sentence.
    pub fn status_message(&self) -> Option<String> {
        match self {
            Self::Stalled(after) => Some(format!(
                "The search model download stalled: no data arrived for {} seconds. Check that Hatchdoor can reach Hugging Face, then retry setup.",
                after.as_secs()
            )),
            Self::DeadlineExceeded(deadline) => Some(format!(
                "The search model download did not finish within {} minutes. Retry setup to continue it from where it stopped.",
                deadline.as_secs() / 60
            )),
            Self::Failed(_) => None,
        }
    }
}

impl std::fmt::Display for FetchFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Stalled(after) => write!(
                f,
                "model download stalled: no data for {}s, download stopped",
                after.as_secs()
            ),
            Self::DeadlineExceeded(deadline) => write!(
                f,
                "model download exceeded its {}s deadline, download stopped",
                deadline.as_secs()
            ),
            Self::Failed(error) => f.write_str(error),
        }
    }
}

enum FetchEvent {
    Progress(u64, u64),
    Error(String),
}

fn parse_fetch_line(line: &str) -> Option<FetchEvent> {
    let rest = line.strip_prefix(FETCH_LINE_PREFIX)?.strip_prefix(' ')?;
    if let Some(error) = rest.strip_prefix("error ") {
        return Some(FetchEvent::Error(error.to_string()));
    }
    let mut fields = rest.strip_prefix("progress ")?.split(' ');
    let downloaded = fields.next()?.parse().ok()?;
    let total = fields.next()?.parse().ok()?;
    Some(FetchEvent::Progress(downloaded, total))
}

/// Run a model download in a child process and stop it when no data has
/// arrived for `limits.stall_after` or the attempt has run for
/// `limits.deadline`. `hf-hub` sets no timeout and offers no way to cancel a
/// request, so a connection that stops answering would otherwise hold the
/// download, and the file lock it took, for as long as Hatchdoor runs (#642).
/// Stopping the process releases both. Partial files stay for the next
/// attempt to resume.
///
/// Blocks until the child ends. `report` receives each progress line.
pub fn run_supervised_fetch(
    mut command: Command,
    limits: FetchLimits,
    report: &dyn Fn(u64, u64),
) -> Result<(), FetchFailure> {
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit());
    let mut child = command
        .spawn()
        .map_err(|error| FetchFailure::Failed(format!("start model download: {error}")))?;
    let stdout = child.stdout.take().expect("stdout is piped");
    let (events, received) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        // By bytes, so one line that is not UTF-8 cannot end the reading
        // early and leave the child unwatched.
        for line in std::io::BufReader::new(stdout).split(b'\n') {
            let Ok(line) = line else { break };
            if let Some(event) = parse_fetch_line(String::from_utf8_lossy(&line).trim_end())
                && events.send(event).is_err()
            {
                break;
            }
        }
    });

    let started = Instant::now();
    let mut last_data = started;
    let mut error = None;
    loop {
        let now = Instant::now();
        let stall_at = last_data + limits.stall_after;
        let deadline_at = started + limits.deadline;
        let wait = stall_at.min(deadline_at).saturating_duration_since(now);
        match received.recv_timeout(wait) {
            Ok(FetchEvent::Progress(downloaded, total)) => {
                last_data = Instant::now();
                report(downloaded, total);
            }
            Ok(FetchEvent::Error(message)) => error = Some(message),
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(if Instant::now() >= deadline_at {
                    FetchFailure::DeadlineExceeded(limits.deadline)
                } else {
                    FetchFailure::Stalled(limits.stall_after)
                });
            }
            // The child closed its output, which it does by exiting.
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }
    let status = child
        .wait()
        .map_err(|error| FetchFailure::Failed(format!("wait for model download: {error}")))?;
    if status.success() {
        Ok(())
    } else {
        Err(FetchFailure::Failed(error.unwrap_or_else(|| {
            format!("model download process ended with {status}")
        })))
    }
}

/// The child side of [`run_supervised_fetch`], run by `hatchdoor
/// --fetch-model <gemma|nomic> <models dir>`: download the model, writing
/// progress and any error to `out` for the supervisor. Returns the process
/// exit code.
pub fn run_fetch_child(args: &[String], out: impl Write + Send + 'static) -> i32 {
    let out = Arc::new(std::sync::Mutex::new(out));
    let emit = {
        let out = out.clone();
        move |line: String| {
            let mut out = out.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
            let _ = writeln!(out, "{FETCH_LINE_PREFIX} {line}");
            let _ = out.flush();
        }
    };
    let (Some(model), Some(models_dir), None) = (args.first(), args.get(1), args.get(2)) else {
        emit(format!(
            "error usage: hatchdoor {FETCH_MODEL_FLAG} <gemma|nomic> <models directory>"
        ));
        return 2;
    };
    let selected = match model.as_str() {
        "gemma" => SelectedModel::Gemma,
        "nomic" => SelectedModel::Nomic,
        other => {
            emit(format!("error unknown model {other}"));
            return 2;
        }
    };
    let last_report = std::sync::Mutex::new(None::<(Instant, u64)>);
    let report = {
        let emit = emit.clone();
        Arc::new(move |downloaded: u64, total: u64| {
            let mut last = last_report
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            // Every start of a file (the count going back) is reported, and
            // otherwise only new data, at most every FETCH_REPORT_EVERY.
            let due = match *last {
                None => true,
                Some((at, previous)) => {
                    downloaded < previous
                        || (downloaded > previous && at.elapsed() >= FETCH_REPORT_EVERY)
                        || downloaded == total && previous != total
                }
            };
            if due {
                *last = Some((Instant::now(), downloaded));
                emit(format!("progress {downloaded} {total}"));
            }
        })
    };
    match ModelSetup::new(PathBuf::from(models_dir)).fetch_model(selected, report) {
        Ok(()) => 0,
        Err(error) => {
            emit(format!("error {}", error.replace('\n', " ")));
            1
        }
    }
}

/// The record a first load wrote before #643, removed when found.
const LEGACY_INTEGRITY_RECORD: &str = "integrity.json";

fn is_local_metadata(path: &Path) -> bool {
    matches!(
        path.file_name().and_then(|name| name.to_str()),
        Some("acceptance.json" | FILES_CHANGED_RECORD)
    )
}

/// Written in a model's folder before its files change, removed once the
/// search index has checked itself against them (#693).
const FILES_CHANGED_RECORD: &str = "files-changed.json";

/// Record, before anything in `dir` changes, that its files are changing.
fn mark_files_changed(dir: &Path) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|error| format!("create model directory: {error}"))?;
    write_json_atomic(
        &dir.join(FILES_CHANGED_RECORD),
        &serde_json::json!({ "changed": true }),
    )
}

fn file_sha256(path: &Path) -> Result<String, String> {
    let mut file = std::fs::File::open(path)
        .map_err(|error| format!("open model artifact {}: {error}", path.display()))?;
    let mut context = ring::digest::Context::new(&ring::digest::SHA256);
    let mut buffer = vec![0_u8; 1024 * 1024];
    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|error| format!("read model artifact {}: {error}", path.display()))?;
        if read == 0 {
            break;
        }
        context.update(&buffer[..read]);
    }
    Ok(context
        .finish()
        .as_ref()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect())
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

/// Replace a setup record so a power cut leaves the old record or the new
/// one, never an empty file: the bytes reach the disk before the rename, and
/// the rename reaches it before this returns (#643).
fn write_json_atomic(path: &Path, value: &impl Serialize) -> Result<(), String> {
    let bytes =
        serde_json::to_vec_pretty(value).map_err(|error| format!("encode json: {error}"))?;
    let temporary = path.with_extension("tmp");
    let written = std::fs::File::create(&temporary).and_then(|mut file| {
        file.write_all(&bytes)?;
        file.sync_all()
    });
    if let Err(error) = written {
        let _ = std::fs::remove_file(&temporary);
        return Err(format!("write {}: {error}", temporary.display()));
    }
    std::fs::rename(&temporary, path)
        .map_err(|error| format!("replace {}: {error}", path.display()))?;
    // The new record is in place either way; a filesystem that cannot
    // flush a folder, as some network shares cannot, only loses that
    // guarantee, so the save still counts as done.
    #[cfg(unix)]
    if let Some(parent) = path.parent()
        && let Err(error) = std::fs::File::open(parent).and_then(|directory| directory.sync_all())
    {
        tracing::warn!("Could not flush {}: {error}", parent.display());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fresh_model_directory_requires_gemma_terms() {
        let temp = tempfile::tempdir().expect("tempdir");
        assert_eq!(
            ModelSetup::new(temp.path().join("models")).selected(),
            SelectedModel::TermsRequired
        );
    }

    #[test]
    fn accepting_gemma_persists_a_local_versioned_receipt() {
        let temp = tempfile::tempdir().expect("tempdir");
        let setup = ModelSetup::new(temp.path().join("models"));
        setup.accept_gemma().expect("accept");
        assert_eq!(setup.selected(), SelectedModel::Gemma);
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
        assert_eq!(setup.selected(), SelectedModel::Nomic);
    }

    /// A stand-in for the Hugging Face file server: every `resolve` request
    /// gets `body` (a per-path ETag, Range honoured), with the headers hf-hub
    /// reads, the commit being the revision the request named. Returns its
    /// address and how many requests it has answered.
    fn serve_model_files(body: &'static [u8]) -> (String, Arc<std::sync::atomic::AtomicUsize>) {
        use std::io::Write as _;
        use std::sync::atomic::{AtomicUsize, Ordering};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
        let address = format!("http://{}", listener.local_addr().expect("address"));
        let requests = Arc::new(AtomicUsize::new(0));
        let counted = requests.clone();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { break };
                let mut reader = std::io::BufReader::new(stream.try_clone().expect("clone"));
                let mut request_line = String::new();
                if reader.read_line(&mut request_line).is_err() {
                    continue;
                }
                let path = request_line.split(' ').nth(1).unwrap_or("").to_string();
                // `/<owner>/<name>/resolve/<revision>/<file>`
                let commit = path.split('/').nth(4).unwrap_or("main").to_string();
                let mut start = 0usize;
                let mut end = body.len() - 1;
                loop {
                    let mut header = String::new();
                    if reader.read_line(&mut header).unwrap_or(0) == 0 || header == "\r\n" {
                        break;
                    }
                    let lower = header.to_ascii_lowercase();
                    if let Some(range) = lower.trim().strip_prefix("range: bytes=") {
                        let (from, to) = range.split_once('-').expect("range");
                        start = from.parse().expect("range start");
                        if !to.is_empty() {
                            end = to.parse().expect("range end");
                        }
                    }
                }
                counted.fetch_add(1, Ordering::SeqCst);
                let slice = &body[start..=end];
                let etag = format!("{:x}", path.bytes().map(u64::from).sum::<u64>());
                let head = format!(
                    "HTTP/1.1 206 Partial Content\r\netag: \"{etag}\"\r\nx-repo-commit: {commit}\r\ncontent-range: bytes {start}-{end}/{}\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
                    body.len(),
                    slice.len()
                );
                let _ = stream.write_all(head.as_bytes());
                let _ = stream.write_all(slice);
            }
        });
        (address, requests)
    }

    fn sha256_hex(bytes: &[u8]) -> String {
        ring::digest::digest(&ring::digest::SHA256, bytes)
            .as_ref()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect()
    }

    /// A two-file model whose known digests are those of `body`, the bytes
    /// [`serve_model_files`] serves, so tests can download and verify it with
    /// no request leaving the machine.
    fn test_model(body: &[u8]) -> PinnedModel {
        let digest: &'static str = Box::leak(sha256_hex(body).into_boxed_str());
        let files: &'static [ModelFile] = Box::leak(Box::new([
            ModelFile {
                path: "onnx/model.onnx",
                sha256: digest,
            },
            ModelFile {
                path: "tokenizer.json",
                sha256: digest,
            },
        ]));
        PinnedModel {
            repository: "test-owner/test-model",
            revision: "1111111111111111111111111111111111111111",
            files,
        }
    }

    const BODY: &[u8] = b"model bytes";

    /// A cache downloaded through `hf-hub`'s own layout: blobs, and links to
    /// them under `snapshots/<revision>`.
    fn downloaded_cache(dir: &Path, model: &PinnedModel) -> PathBuf {
        let (endpoint, _) = serve_model_files(BODY);
        fetch_pinned(
            dir,
            model,
            &DownloadSource { endpoint },
            Arc::new(|_, _| {}),
        )
        .expect("fetch");
        let snapshot = snapshot_dir(dir, model);
        assert_eq!(
            verify_snapshot(&snapshot, model).unwrap(),
            CacheIntegrity::Valid
        );
        snapshot
    }

    /// Every file both models load has a SHA-256 shipped with Hatchdoor.
    #[test]
    fn both_models_ship_a_digest_for_every_file() {
        for model in [&NOMIC, &GEMMA] {
            assert_eq!(model.revision.len(), 40, "{}", model.repository);
            for file in model.files {
                assert_eq!(file.sha256.len(), 64, "{}", file.path);
                assert!(
                    file.sha256
                        .bytes()
                        .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)),
                    "{}",
                    file.path
                );
            }
        }
    }

    /// With no record of earlier loads at all, a file with one byte changed
    /// is refused against the shipped digest, and removed so it is fetched
    /// again (#643, SB-06).
    #[test]
    fn a_changed_byte_is_refused_without_any_earlier_record() {
        let temp = tempfile::tempdir().expect("tempdir");
        let model = test_model(BODY);
        let snapshot = downloaded_cache(temp.path(), &model);
        let tokenizer = snapshot.join("tokenizer.json");
        std::fs::write(&tokenizer, b"model bytez").expect("change a byte");

        assert_eq!(
            verify_snapshot(&snapshot, &model).unwrap(),
            CacheIntegrity::Invalid
        );
        assert_eq!(
            prepare_cache(temp.path(), &model, 0).unwrap(),
            CacheIntegrity::Missing
        );
        assert!(!snapshot.exists(), "the changed cache was removed");
    }

    /// A snapshot link replaced by a regular file is checked like the link:
    /// by its bytes. A file the model does not list makes the cache invalid
    /// too, and so does a link to nothing (#643, SB-07).
    #[test]
    fn snapshot_entries_are_checked_through_the_paths_the_load_opens() {
        let temp = tempfile::tempdir().expect("tempdir");
        let model = test_model(BODY);
        let snapshot = downloaded_cache(temp.path(), &model);
        let tokenizer = snapshot.join("tokenizer.json");
        assert!(
            tokenizer
                .symlink_metadata()
                .unwrap()
                .file_type()
                .is_symlink()
        );

        std::fs::remove_file(&tokenizer).expect("remove link");
        std::fs::write(&tokenizer, b"lowercase: false").expect("regular file");
        assert_eq!(
            verify_snapshot(&snapshot, &model).unwrap(),
            CacheIntegrity::Invalid
        );

        std::fs::write(&tokenizer, BODY).expect("same bytes");
        assert_eq!(
            verify_snapshot(&snapshot, &model).unwrap(),
            CacheIntegrity::Valid
        );

        std::fs::write(snapshot.join("added.json"), b"{}").expect("unlisted file");
        assert_eq!(
            verify_snapshot(&snapshot, &model).unwrap(),
            CacheIntegrity::Invalid
        );
        std::fs::remove_file(snapshot.join("added.json")).expect("remove unlisted");
        std::fs::write(snapshot.join(".DS_Store"), b"finder").expect("hidden file");
        assert_eq!(
            verify_snapshot(&snapshot, &model).unwrap(),
            CacheIntegrity::Valid
        );

        #[cfg(unix)]
        {
            std::fs::remove_file(&tokenizer).expect("remove file");
            std::os::unix::fs::symlink("../../blobs/gone", &tokenizer).expect("dangling link");
            assert_eq!(
                verify_snapshot(&snapshot, &model).unwrap(),
                CacheIntegrity::Invalid
            );
        }
    }

    /// A partial download is missing, not invalid, so it resumes.
    #[test]
    fn a_partial_download_is_missing_and_resumes() {
        let temp = tempfile::tempdir().expect("tempdir");
        let model = test_model(BODY);
        assert_eq!(
            verify_snapshot(&snapshot_dir(temp.path(), &model), &model).unwrap(),
            CacheIntegrity::Missing
        );
        let snapshot = downloaded_cache(temp.path(), &model);
        std::fs::remove_file(snapshot.join("tokenizer.json")).expect("remove");
        assert_eq!(
            verify_snapshot(&snapshot, &model).unwrap(),
            CacheIntegrity::Missing
        );
        assert!(snapshot.join("onnx/model.onnx").exists());
    }

    /// The record older versions kept is removed, empty or not, and never
    /// stops setup: a damaged file is fetched again through the test double
    /// and the cache ends valid (#643, SB-04).
    #[test]
    fn an_empty_legacy_record_does_not_stop_a_refetch() {
        let temp = tempfile::tempdir().expect("tempdir");
        let model = test_model(BODY);
        let snapshot = downloaded_cache(temp.path(), &model);
        std::fs::write(temp.path().join(LEGACY_INTEGRITY_RECORD), b"").expect("empty record");
        std::fs::write(snapshot.join("onnx/model.onnx"), b"truncated").expect("damage");

        assert_eq!(
            prepare_cache(temp.path(), &model, 0).unwrap(),
            CacheIntegrity::Missing
        );
        assert!(!temp.path().join(LEGACY_INTEGRITY_RECORD).exists());
        let (endpoint, _) = serve_model_files(BODY);
        fetch_pinned(
            temp.path(),
            &model,
            &DownloadSource { endpoint },
            Arc::new(|_, _| {}),
        )
        .expect("refetch");
        assert_eq!(
            prepare_cache(temp.path(), &model, 0).unwrap(),
            CacheIntegrity::Valid
        );
    }

    /// An empty selection or terms receipt, as a power cut mid-write left
    /// before records were flushed, asks for the choice again (#643, SB-04).
    #[test]
    fn damaged_choice_records_ask_for_the_choice_again() {
        let temp = tempfile::tempdir().expect("tempdir");
        let setup = ModelSetup::new(temp.path().join("models"));
        std::fs::create_dir_all(setup.models_dir()).expect("models dir");
        std::fs::write(setup.models_dir().join("selection.json"), b"").expect("empty");
        assert_eq!(setup.selected(), SelectedModel::TermsRequired);

        setup.accept_gemma().expect("accept");
        assert_eq!(setup.selected(), SelectedModel::Gemma);
        let receipt = setup
            .model_cache_dir(SelectedModel::Gemma)
            .join("acceptance.json");
        std::fs::write(&receipt, b"").expect("empty receipt");
        assert_eq!(setup.selected(), SelectedModel::TermsRequired);
    }

    /// The free-space floor guards downloads only: a valid cache is used
    /// whatever the free space, and a cache that needs a download is still
    /// refused below the floor (#643, SB-05).
    #[test]
    fn the_free_space_floor_applies_only_to_a_download() {
        let temp = tempfile::tempdir().expect("tempdir");
        let model = test_model(BODY);
        downloaded_cache(temp.path(), &model);
        assert_eq!(
            prepare_cache(temp.path(), &model, u64::MAX).unwrap(),
            CacheIntegrity::Valid
        );

        let empty = tempfile::tempdir().expect("tempdir");
        let refused = prepare_cache(empty.path(), &model, u64::MAX).expect_err("no room");
        assert!(refused.contains("not enough free space"), "{refused}");
    }

    /// Nomic downloads at its pinned revision into the snapshot setup checks,
    /// and a second setup fetches nothing (#642, #643).
    #[test]
    fn nomic_fetch_fills_the_pinned_snapshot() {
        use std::sync::atomic::Ordering;
        let temp = tempfile::tempdir().expect("tempdir");
        let setup = ModelSetup::new(temp.path().join("models"));
        let (endpoint, requests) = serve_model_files(BODY);
        let source = DownloadSource { endpoint };
        let reported = Arc::new(std::sync::Mutex::new(Vec::new()));
        let report = {
            let reported = reported.clone();
            Arc::new(move |downloaded, total| reported.lock().unwrap().push((downloaded, total)))
        };

        setup
            .fetch_model_from(SelectedModel::Nomic, &source, report.clone())
            .expect("fetch");

        let snapshot = setup.snapshot_dir(SelectedModel::Nomic).expect("snapshot");
        assert!(snapshot.ends_with(format!("snapshots/{NOMIC_REVISION}")));
        for file in NOMIC.files {
            assert_eq!(
                std::fs::read(snapshot.join(file.path)).expect("read"),
                BODY,
                "{}",
                file.path
            );
        }
        assert!(reported.lock().unwrap().contains(&(11, 11)));
        // The test double's bytes are not Nomic's.
        assert_eq!(
            setup.verify_cache(SelectedModel::Nomic).unwrap(),
            CacheIntegrity::Invalid
        );
        assert!(setup.verify_download(SelectedModel::Nomic).is_err());

        let answered = requests.load(Ordering::SeqCst);
        setup
            .fetch_model_from(SelectedModel::Nomic, &source, report)
            .expect("second fetch");
        assert_eq!(requests.load(Ordering::SeqCst), answered);

        setup
            .point_main_at_pinned_revision(SelectedModel::Nomic)
            .expect("ref");
        let cached = Cache::new(setup.model_cache_dir(SelectedModel::Nomic))
            .repo(Repo::model(NOMIC_REPOSITORY.to_string()));
        assert_eq!(
            cached.get("tokenizer.json"),
            Some(snapshot.join("tokenizer.json")),
            "FastEmbed's `main` lookup finds the pinned snapshot"
        );
    }

    /// #693: every change of model files is recorded on disk before it
    /// happens and survives until the index has checked itself, so a start
    /// that stops in between still counts as a change at the next one.
    #[test]
    fn a_change_of_model_files_is_recorded_until_the_index_has_checked() {
        let temp = tempfile::tempdir().expect("tempdir");
        let setup = ModelSetup::new(temp.path().join("models"));
        let selected = SelectedModel::Nomic;
        assert!(!setup.files_changed_since_index(selected));

        assert_eq!(
            setup.prepare_download(selected).expect("prepare"),
            CacheIntegrity::Missing
        );
        assert!(
            setup.files_changed_since_index(selected),
            "a download follows"
        );
        setup.files_reached_index(selected).expect("clear");
        assert!(!setup.files_changed_since_index(selected));
        setup
            .files_reached_index(selected)
            .expect("clearing twice is fine");

        setup
            .point_main_at_pinned_revision(selected)
            .expect("point main");
        assert!(setup.files_changed_since_index(selected), "main moved");
        setup.files_reached_index(selected).expect("clear");
        setup
            .point_main_at_pinned_revision(selected)
            .expect("main already right");
        assert!(
            !setup.files_changed_since_index(selected),
            "a ref already right changes nothing"
        );

        setup.reset_download_cache(selected).expect("reset");
        assert!(
            setup.files_changed_since_index(selected),
            "the record survives the reset it announces"
        );
    }

    /// A `main` already naming the pinned commit is not written again, so a
    /// valid cache on a volume that takes no writes still loads.
    #[cfg(unix)]
    #[test]
    fn a_correct_main_ref_is_not_rewritten() {
        use std::os::unix::fs::PermissionsExt;
        let temp = tempfile::tempdir().expect("tempdir");
        let setup = ModelSetup::new(temp.path().join("models"));
        setup
            .point_main_at_pinned_revision(SelectedModel::Nomic)
            .expect("first write");
        let refs = setup
            .model_cache_dir(SelectedModel::Nomic)
            .join(Repo::model(NOMIC_REPOSITORY.to_string()).folder_name())
            .join("refs");
        std::fs::set_permissions(&refs, std::fs::Permissions::from_mode(0o555)).expect("chmod");
        std::fs::set_permissions(refs.join("main"), std::fs::Permissions::from_mode(0o444))
            .expect("chmod");
        let result = setup.point_main_at_pinned_revision(SelectedModel::Nomic);
        std::fs::set_permissions(&refs, std::fs::Permissions::from_mode(0o755)).expect("chmod");
        result.expect("nothing to write");
    }

    /// A Nomic snapshot FastEmbed filled from `main` before #643, at the
    /// commit now pinned, has no `refs/<commit>`; the fetch still finds its
    /// files and sends no request. (Their bytes are checked separately.)
    #[test]
    fn a_cache_fastembed_filled_from_main_needs_no_download() {
        let temp = tempfile::tempdir().expect("tempdir");
        let setup = ModelSetup::new(temp.path().join("models"));
        let snapshot = setup.snapshot_dir(SelectedModel::Nomic).expect("snapshot");
        for file in NOMIC.files {
            let path = snapshot.join(file.path);
            std::fs::create_dir_all(path.parent().unwrap()).expect("dir");
            std::fs::write(path, b"present").expect("file");
        }
        let closed = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
        let endpoint = format!("http://{}", closed.local_addr().expect("address"));
        drop(closed);
        setup
            .fetch_model_from(
                SelectedModel::Nomic,
                &DownloadSource { endpoint },
                Arc::new(|_, _| {}),
            )
            .expect("nothing to fetch");
    }

    #[test]
    fn records_are_replaced_whole() {
        let temp = tempfile::tempdir().expect("tempdir");
        let path = temp.path().join("selection.json");
        write_json_atomic(
            &path,
            &SelectionRecord {
                selected: "nomic".into(),
            },
        )
        .expect("write");
        write_json_atomic(
            &path,
            &SelectionRecord {
                selected: "gemma".into(),
            },
        )
        .expect("rewrite");
        let record: SelectionRecord =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(record.selected, "gemma");
        assert!(!path.with_extension("tmp").exists());
    }

    #[test]
    fn fetch_lines_parse_progress_and_errors_only() {
        assert!(matches!(
            parse_fetch_line("hatchdoor-model-fetch progress 5 10"),
            Some(FetchEvent::Progress(5, 10))
        ));
        assert!(matches!(
            parse_fetch_line("hatchdoor-model-fetch error download failed: 404"),
            Some(FetchEvent::Error(message)) if message == "download failed: 404"
        ));
        assert!(parse_fetch_line("2026-11-06 INFO Hatchdoor 2.9.0").is_none());
        assert!(parse_fetch_line("hatchdoor-model-fetch progress five 10").is_none());
    }

    #[cfg(unix)]
    fn shell(script: &str) -> Command {
        let mut command = Command::new("sh");
        command.arg("-c").arg(script);
        command
    }

    #[cfg(unix)]
    fn limits(stall_ms: u64, deadline_ms: u64) -> FetchLimits {
        FetchLimits {
            stall_after: Duration::from_millis(stall_ms),
            deadline: Duration::from_millis(deadline_ms),
        }
    }

    /// A download that stops sending data is stopped once the stall bound
    /// passes, not left running (#642).
    #[cfg(unix)]
    #[test]
    fn a_download_that_stops_moving_is_stopped_as_stalled() {
        let reported = std::sync::Mutex::new(Vec::new());
        let started = Instant::now();
        let result = run_supervised_fetch(
            shell("echo 'hatchdoor-model-fetch progress 1 10'; exec sleep 30"),
            limits(300, 30_000),
            &|downloaded, total| reported.lock().unwrap().push((downloaded, total)),
        );
        assert_eq!(
            result,
            Err(FetchFailure::Stalled(Duration::from_millis(300)))
        );
        assert!(started.elapsed() < Duration::from_secs(10));
        assert_eq!(*reported.lock().unwrap(), vec![(1, 10)]);
    }

    /// Data that keeps trickling in cannot hold setup past the deadline.
    #[cfg(unix)]
    #[test]
    fn a_trickling_download_is_stopped_at_its_deadline() {
        let result = run_supervised_fetch(
            shell(
                "i=0; while true; do i=$((i+1)); echo \"hatchdoor-model-fetch progress $i 1000\"; sleep 0.05; done",
            ),
            limits(5_000, 400),
            &|_, _| {},
        );
        assert_eq!(
            result,
            Err(FetchFailure::DeadlineExceeded(Duration::from_millis(400)))
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_finished_download_succeeds_and_a_failed_one_carries_its_error() {
        let reported = std::sync::Mutex::new(Vec::new());
        assert_eq!(
            run_supervised_fetch(
                shell("echo 'hatchdoor-model-fetch progress 10 10'; echo 'a log line'"),
                limits(5_000, 30_000),
                &|downloaded, total| reported.lock().unwrap().push((downloaded, total)),
            ),
            Ok(())
        );
        assert_eq!(*reported.lock().unwrap(), vec![(10, 10)]);

        assert_eq!(
            run_supervised_fetch(
                shell(
                    "echo 'hatchdoor-model-fetch error download Nomic artifact config.json: 404'; exit 1"
                ),
                limits(5_000, 30_000),
                &|_, _| {},
            ),
            Err(FetchFailure::Failed(
                "download Nomic artifact config.json: 404".to_string()
            ))
        );
    }

    #[test]
    fn a_stopped_download_names_itself_in_the_status_message() {
        let stalled = FetchFailure::Stalled(DOWNLOAD_STALL_AFTER)
            .status_message()
            .expect("message");
        assert!(stalled.contains("stalled"), "{stalled}");
        assert!(stalled.contains("60 seconds"), "{stalled}");
        let late = FetchFailure::DeadlineExceeded(DOWNLOAD_DEADLINE)
            .status_message()
            .expect("message");
        assert!(late.contains("120 minutes"), "{late}");
        assert!(
            FetchFailure::Failed("404".into())
                .status_message()
                .is_none()
        );
    }

    const FETCH_CHILD_DIR: &str = "HATCHDOOR_TEST_FETCH_CHILD_DIR";

    /// Not a test by itself: the child half of
    /// `a_real_download_from_a_server_that_never_answers_is_stopped`, run in
    /// a copy of this test binary. Does nothing when run normally.
    #[test]
    fn fetch_child_for_supervisor_test() {
        if let Ok(dir) = std::env::var(FETCH_CHILD_DIR) {
            let code = run_fetch_child(&["nomic".to_string(), dir], std::io::stdout());
            std::process::exit(code);
        }
    }

    /// The real download path, `hf-hub` included, against a server that
    /// accepts the connection and never answers: the child is stopped as
    /// stalled. No request leaves the machine.
    #[test]
    fn a_real_download_from_a_server_that_never_answers_is_stopped() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
        let endpoint = format!("http://{}", listener.local_addr().expect("address"));
        let held = Arc::new(std::sync::Mutex::new(Vec::new()));
        {
            let held = held.clone();
            std::thread::spawn(move || {
                for stream in listener.incoming().flatten() {
                    held.lock().unwrap().push(stream);
                }
            });
        }
        let temp = tempfile::tempdir().expect("tempdir");
        let mut command = Command::new(std::env::current_exe().expect("test binary"));
        command
            .args([
                "--exact",
                "model_setup::tests::fetch_child_for_supervisor_test",
                "--nocapture",
                "--test-threads=1",
            ])
            .env(FETCH_CHILD_DIR, temp.path().join("models"))
            .env("HF_ENDPOINT", &endpoint)
            .env_remove("HF_HOME");

        let started = Instant::now();
        let result = run_supervised_fetch(command, limits_for_real_child(), &|_, _| {});

        assert_eq!(result, Err(FetchFailure::Stalled(Duration::from_secs(2))));
        assert!(started.elapsed() < Duration::from_secs(30));
        assert!(
            !held.lock().unwrap().is_empty(),
            "the child reached the server before it was stopped"
        );
    }

    /// The download child ignores `HF_HOME`: the files land in the models
    /// folder, where setup checks and loads them (#643, SB-03).
    #[test]
    fn a_download_with_hf_home_set_still_fills_the_models_folder() {
        let temp = tempfile::tempdir().expect("tempdir");
        let hf_home = temp.path().join("hf-home");
        std::fs::create_dir_all(&hf_home).expect("hf home");
        let (endpoint, _) = serve_model_files(BODY);
        let mut command = Command::new(std::env::current_exe().expect("test binary"));
        command
            .args([
                "--exact",
                "model_setup::tests::fetch_child_for_supervisor_test",
                "--nocapture",
                "--test-threads=1",
            ])
            .env(FETCH_CHILD_DIR, temp.path().join("models"))
            .env("HF_ENDPOINT", &endpoint)
            .env("HF_HOME", &hf_home);

        run_supervised_fetch(command, limits_for_real_child(), &|_, _| {}).expect("fetch");

        let setup = ModelSetup::new(temp.path().join("models"));
        let snapshot = setup.snapshot_dir(SelectedModel::Nomic).expect("snapshot");
        for file in NOMIC.files {
            assert!(snapshot.join(file.path).is_file(), "{}", file.path);
        }
        assert_eq!(std::fs::read_dir(&hf_home).unwrap().count(), 0);
    }

    fn limits_for_real_child() -> FetchLimits {
        FetchLimits {
            stall_after: Duration::from_secs(2),
            deadline: Duration::from_secs(60),
        }
    }
}
