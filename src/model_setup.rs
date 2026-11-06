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
/// What FastEmbed reads for Nomic v1.5 (`EmbeddingModel::NomicEmbedTextV15`
/// in fastembed 5.17): the ONNX graph and the four tokenizer files. Fetched
/// up front so the load that follows finds them all on disk.
const NOMIC_FILES: [&str; 5] = [
    "onnx/model.onnx",
    "config.json",
    "special_tokens_map.json",
    "tokenizer.json",
    "tokenizer_config.json",
];
const GEMMA_FILES: [&str; 6] = [
    "onnx/model_q4.onnx",
    "onnx/model_q4.onnx_data",
    "config.json",
    "special_tokens_map.json",
    "tokenizer.json",
    "tokenizer_config.json",
];
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
            hf_home: std::env::var("HF_HOME").ok().map(PathBuf::from),
        };
        self.fetch_model_from(selected, &source, report)
    }

    fn fetch_model_from(
        &self,
        selected: SelectedModel,
        source: &DownloadSource,
        report: Arc<dyn Fn(u64, u64) + Send + Sync>,
    ) -> Result<(), String> {
        match selected {
            SelectedModel::Gemma => self.fetch_gemma_at_pinned_revision(source, report),
            SelectedModel::Nomic => self.fetch_nomic(source, report),
            SelectedModel::TermsRequired => Err("model terms have not been accepted".to_string()),
        }
    }

    /// Download every artifact FastEmbed needs through a fixed Gemma revision,
    /// then point FastEmbed's normal `main` cache lookup at that immutable
    /// snapshot. FastEmbed itself does not expose a revision option.
    fn fetch_gemma_at_pinned_revision(
        &self,
        source: &DownloadSource,
        report: Arc<dyn Fn(u64, u64) + Send + Sync>,
    ) -> Result<(), String> {
        let cache_dir = self.model_cache_dir(SelectedModel::Gemma);
        let api = source.client(cache_dir.clone())?;
        let pinned_repo = Repo::with_revision(
            GEMMA_REPOSITORY.to_string(),
            RepoType::Model,
            GEMMA_REVISION.to_string(),
        );
        let cached = Cache::new(cache_dir.clone()).repo(pinned_repo.clone());
        let pinned = api.repo(pinned_repo);
        for file in GEMMA_FILES {
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

    /// Download Nomic's files into the cache FastEmbed's own lookup reads:
    /// `HF_HOME` when set, as FastEmbed resolves it, else the model's
    /// directory, at revision `main`.
    fn fetch_nomic(
        &self,
        source: &DownloadSource,
        report: Arc<dyn Fn(u64, u64) + Send + Sync>,
    ) -> Result<(), String> {
        let cache_dir = source
            .hf_home
            .clone()
            .unwrap_or_else(|| self.model_cache_dir(SelectedModel::Nomic));
        let api = source.client(cache_dir.clone())?;
        let repo = Repo::model(NOMIC_REPOSITORY.to_string());
        let cached = Cache::new(cache_dir).repo(repo.clone());
        let remote = api.repo(repo);
        for file in NOMIC_FILES {
            if cached.get(file).is_none() {
                remote
                    .download_with_progress(file, ModelDownloadProgress::new(report.clone()))
                    .map_err(|error| format!("download Nomic artifact {file}: {error}"))?;
            }
        }
        Ok(())
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

/// Where model files come from: `HF_ENDPOINT` and `HF_HOME` as FastEmbed's
/// own lookup reads them, so the load finds what the download wrote.
struct DownloadSource {
    endpoint: String,
    hf_home: Option<PathBuf>,
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

    /// A stand-in for the Hugging Face file server: every `resolve` request
    /// gets `body` (a per-path ETag, Range honoured), with the headers hf-hub
    /// reads. Returns its address and how many requests it has answered.
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
                    "HTTP/1.1 206 Partial Content\r\netag: \"{etag}\"\r\nx-repo-commit: 0123456789abcdef\r\ncontent-range: bytes {start}-{end}/{}\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
                    body.len(),
                    slice.len()
                );
                let _ = stream.write_all(head.as_bytes());
                let _ = stream.write_all(slice);
            }
        });
        (address, requests)
    }

    /// Nomic's files land where FastEmbed's own lookup (`Repo::model`,
    /// revision `main`) finds them, so its load needs no network, and a
    /// second setup fetches nothing (#642).
    #[test]
    fn nomic_fetch_fills_the_cache_fastembed_reads() {
        use std::sync::atomic::Ordering;
        let temp = tempfile::tempdir().expect("tempdir");
        let setup = ModelSetup::new(temp.path().join("models"));
        let (endpoint, requests) = serve_model_files(b"model bytes");
        let source = DownloadSource {
            endpoint,
            hf_home: None,
        };
        let reported = Arc::new(std::sync::Mutex::new(Vec::new()));
        let report = {
            let reported = reported.clone();
            Arc::new(move |downloaded, total| reported.lock().unwrap().push((downloaded, total)))
        };

        setup
            .fetch_model_from(SelectedModel::Nomic, &source, report.clone())
            .expect("fetch");

        let cached = Cache::new(setup.model_cache_dir(SelectedModel::Nomic))
            .repo(Repo::model(NOMIC_REPOSITORY.to_string()));
        for file in NOMIC_FILES {
            let path = cached.get(file).unwrap_or_else(|| panic!("{file} cached"));
            assert_eq!(std::fs::read(path).expect("read"), b"model bytes");
        }
        assert!(reported.lock().unwrap().contains(&(11, 11)));

        let answered = requests.load(Ordering::SeqCst);
        setup
            .fetch_model_from(SelectedModel::Nomic, &source, report)
            .expect("second fetch");
        assert_eq!(requests.load(Ordering::SeqCst), answered);
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

    fn limits_for_real_child() -> FetchLimits {
        FetchLimits {
            stall_after: Duration::from_secs(2),
            deadline: Duration::from_secs(60),
        }
    }
}
