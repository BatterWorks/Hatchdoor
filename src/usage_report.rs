//! The opt-in usage report (ADR-45). This is telemetry, and it is off unless
//! the operator turns it on.
//!
//! While `HATCHDOOR_USAGE_REPORT_ENABLED` is on, the install keeps a random
//! install ID and a small activity record in the instance state file, under
//! its own `usage_report` section, and [`current_report`] builds the report
//! the next send would carry. Settings shows that report whether the setting
//! is on or off.
//!
//! A background task, [`spawn`], posts that report to the collector at most
//! once a day while the setting is on. It reads the setting on every tick, so
//! turning it on sends a first report within a minute and turning it off stops
//! at once, both without a restart. A failed send is tried again an hour
//! later and nothing is kept for later: a report describes the present. A
//! demo instance never starts the task. The request itself sits behind
//! [`SendReport`], so tests replace it and never reach the network.
//!
//! [`UsageReport`] is the one owner of the section. It is brought in line
//! with the setting at startup and after every settings save: on creates an
//! ID if none exists, off clears the whole section, so switching back on
//! starts a new identity. A demo instance keeps nothing, even with the
//! variable set.
//!
//! Every value in the report is a fixed word, a yes or no, or a bucket. The
//! name an MCP client gives itself is mapped onto a closed list of agent
//! families the moment it arrives and is never stored or sent.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

use chrono::NaiveDate;
use serde::{Deserialize, Serialize};

use crate::app_state::AppState;
use crate::instance_state::InstanceStateStore;
use crate::model_setup::SelectedModel;
use crate::runtime_config::{ConfigSnapshot, RuntimeConfig, is_truthy};
use crate::vault_registry::{VaultGitMode, VaultRegistryState, VaultSource};

/// The live setting that turns the report on. Off by default (ADR-45).
pub const USAGE_REPORT_SETTING: &str = "HATCHDOOR_USAGE_REPORT_ENABLED";

/// The instance state section this module owns.
const USAGE_REPORT_SECTION: &str = "usage_report";

/// The report format. A field added, or an agent family added, is a new
/// schema (ADR-45).
const REPORT_SCHEMA: u32 = 1;

/// The collector's website for Hatchdoor installs, in the event format of the
/// analytics service the reports go to (ADR-45).
const COLLECTOR_WEBSITE_ID: &str = "7583bdb0-bd67-4203-8fc3-2a017c5a611d";

/// Where the report goes (ADR-45). The name is public on purpose and is used
/// for nothing else, so an operator who sees it in a network log can tell
/// what it is and block it. No setting, variable or flag changes it.
const COLLECTOR_URL: &str = "https://telemetry-hatchdoor.battercloud.cc/v1/report";

/// Sent without a version, as the update check's is (ADR-39): the report
/// carries the version itself, where Settings shows it.
const USER_AGENT: &str = "Hatchdoor";

/// How long the whole request may take before it counts as a failure.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);

/// How long one successful report stands for its install ID.
pub const REPORT_INTERVAL: Duration = Duration::from_secs(24 * 60 * 60);

/// How long after a failed send the next one is tried.
pub const RETRY_INTERVAL: Duration = Duration::from_secs(60 * 60);

/// How often the task wakes to read the setting. Bounds how soon the first
/// report goes after the setting is turned on, and is the least time between
/// two attempts whatever else happens.
pub const TICK_INTERVAL: Duration = Duration::from_secs(60);

/// Taken off the waits this process times itself, the minute between
/// attempts and the hour before a retry. Ticks come a minute apart give or
/// take how late the timer woke, so a wait of exactly one tick would skip
/// the tick that lands a moment short of it and double the wait.
const TIMER_SLACK: Duration = Duration::from_secs(1);

/// Shown where the install ID would go while the report is off, or while an
/// ID could not be saved.
pub const INSTALL_ID_PLACEHOLDER: &str = "(created when the usage report is turned on)";

/// How far back `mcp_active_7d` and `web_active_7d` look, in days.
const ACTIVE_WINDOW_DAYS: i64 = 7;

/// How far back an agent family counts as seen, in days.
const AGENT_WINDOW_DAYS: i64 = 30;

/// The closed list of agent families (schema 1). `Other` is everything else.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AgentFamily {
    ClaudeCode,
    ClaudeDesktop,
    Codex,
    Cursor,
    VsCode,
    ChatGpt,
    OpenClaw,
    Hermes,
    Other,
}

impl AgentFamily {
    const ALL: [AgentFamily; 9] = [
        AgentFamily::ClaudeCode,
        AgentFamily::ClaudeDesktop,
        AgentFamily::Codex,
        AgentFamily::Cursor,
        AgentFamily::VsCode,
        AgentFamily::ChatGpt,
        AgentFamily::OpenClaw,
        AgentFamily::Hermes,
        AgentFamily::Other,
    ];

    /// The family's name in the stored record; the report field is
    /// `agent_` plus this.
    fn key(self) -> &'static str {
        match self {
            AgentFamily::ClaudeCode => "claude_code",
            AgentFamily::ClaudeDesktop => "claude_desktop",
            AgentFamily::Codex => "codex",
            AgentFamily::Cursor => "cursor",
            AgentFamily::VsCode => "vscode",
            AgentFamily::ChatGpt => "chatgpt",
            AgentFamily::OpenClaw => "openclaw",
            AgentFamily::Hermes => "hermes",
            AgentFamily::Other => "other",
        }
    }

    /// The family of the client that sends `client_name` as its
    /// `clientInfo.name`. The names are what each client sends today:
    /// `claude-code`; `claude-ai` from Claude Desktop and
    /// `Anthropic/ClaudeAI` from the Claude apps' connectors;
    /// `codex-mcp-client`; `cursor-vscode`; `Visual Studio Code`, with a
    /// suffix on Insiders builds; `openai-mcp` and `ChatGPT`. OpenClaw and
    /// Hermes are matched on their own name as a prefix.
    fn of(client_name: &str) -> Self {
        let name = client_name.trim().to_ascii_lowercase();
        match name.as_str() {
            "claude-code" => AgentFamily::ClaudeCode,
            "claude-ai" | "anthropic/claudeai" => AgentFamily::ClaudeDesktop,
            "cursor-vscode" | "cursor" => AgentFamily::Cursor,
            "openai-mcp" | "chatgpt" => AgentFamily::ChatGpt,
            _ if name.starts_with("codex") => AgentFamily::Codex,
            _ if name.starts_with("visual studio code") => AgentFamily::VsCode,
            _ if name.starts_with("openclaw") => AgentFamily::OpenClaw,
            _ if name.starts_with("hermes") => AgentFamily::Hermes,
            _ => AgentFamily::Other,
        }
    }
}

/// The `usage_report` section of the instance state file. Days are UTC
/// calendar days, `2026-10-07`: what the install was seen doing is kept to no
/// finer precision.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
struct UsageReportRecord {
    install_id: String,
    /// The last day an MCP tool call arrived.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    mcp_seen: Option<String>,
    /// The last day a web request arrived.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    web_seen: Option<String>,
    /// The last day each agent family was seen, by [`AgentFamily::key`].
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    agents_seen: BTreeMap<String, String>,
    /// When the collector last accepted a report with this ID. RFC 3339,
    /// seconds, UTC.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    last_sent: Option<String>,
}

fn rfc3339(at: SystemTime) -> String {
    chrono::DateTime::<chrono::Utc>::from(at).to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

fn parse_rfc3339(value: &str) -> Option<SystemTime> {
    chrono::DateTime::parse_from_rfc3339(value)
        .ok()
        .map(SystemTime::from)
}

/// Whether `last` is less than `interval` before `now`. A time in the future,
/// from a clock that moved back, is not: otherwise it could hold reports off
/// for as long as the clock was wrong.
fn within(last: Option<SystemTime>, now: SystemTime, interval: Duration) -> bool {
    last.is_some_and(|last| {
        now.duration_since(last)
            .is_ok_and(|elapsed| elapsed < interval)
    })
}

fn day_of(at: SystemTime) -> NaiveDate {
    chrono::DateTime::<chrono::Utc>::from(at).date_naive()
}

/// Whether `seen` is one of the `window` days ending on `today`. A day in the
/// future, from a clock that moved back, is not.
fn seen_within(seen: Option<&String>, today: NaiveDate, window: i64) -> bool {
    seen.and_then(|day| NaiveDate::parse_from_str(day, "%Y-%m-%d").ok())
        .is_some_and(|day| (0..window).contains(&(today - day).num_days()))
}

/// A random UUID, version 4, derived from nothing on the machine.
fn new_install_id() -> Result<String, String> {
    let mut bytes = [0_u8; 16];
    getrandom::fill(&mut bytes).map_err(|error| error.to_string())?;
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    let hex: String = bytes.iter().map(|byte| format!("{byte:02x}")).collect();
    Ok(format!(
        "{}-{}-{}-{}-{}",
        &hex[0..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..32]
    ))
}

fn is_install_id(value: &str) -> bool {
    let bytes = value.as_bytes();
    bytes.len() == 36
        && bytes.iter().enumerate().all(|(index, byte)| match index {
            8 | 13 | 18 | 23 => *byte == b'-',
            _ => byte.is_ascii_hexdigit(),
        })
}

fn setting_on(snapshot: &ConfigSnapshot, key: &str) -> bool {
    snapshot
        .setting(key)
        .is_some_and(|setting| is_truthy(&setting.value))
}

fn setting_enabled(snapshot: &ConfigSnapshot) -> bool {
    setting_on(snapshot, USAGE_REPORT_SETTING)
}

/// The install ID to show or send under `snapshot`: the saved one, and only
/// while the report is on.
fn usable_install_id(state: &AppState, snapshot: &ConfigSnapshot) -> Option<String> {
    setting_enabled(snapshot)
        .then(|| state.usage_report.install_id())
        .flatten()
}

/// The one owner of the `usage_report` section.
///
/// It holds the saved record in memory while the report is on, so the
/// activity hooks cost a lock and a comparison, and write the file at most
/// once a day per fact. [`Default`] is inert: it keeps nothing and writes
/// nothing, which is also what a demo instance gets.
#[derive(Default)]
pub struct UsageReport {
    backing: Option<Backing>,
    state: Mutex<SectionState>,
}

struct Backing {
    store: InstanceStateStore,
    runtime_config: RuntimeConfig,
}

#[derive(Default)]
struct SectionState {
    /// The record as last saved, or `None` while the report is off or its ID
    /// could not be saved. Only a saved ID is ever shown or used.
    record: Option<UsageReportRecord>,
    /// The section could not be cleared when the report was switched off, so
    /// what the file holds must not come back as this install's identity.
    /// Kept in memory only: a state file that cannot be written cannot hold
    /// the mark either, and the next start clears the section if it is off.
    stale_on_disk: bool,
}

impl UsageReport {
    /// A handle on `store`, which must be a clone of the store every other
    /// section is written through so their writes share its lock. In demo
    /// mode the handle is inert. Call [`Self::reconcile`] before use.
    pub fn new(store: InstanceStateStore, runtime_config: RuntimeConfig, demo_mode: bool) -> Self {
        Self {
            backing: (!demo_mode).then_some(Backing {
                store,
                runtime_config,
            }),
            state: Mutex::default(),
        }
    }

    /// Bring the section in line with the setting as it resolves now: on
    /// creates an install ID if none exists, off clears the whole section.
    /// Runs at startup and after every settings save, because the setting can
    /// change through the environment at a restart as well as through a save.
    ///
    /// Never fails. An ID that cannot be saved is logged and not kept, so it
    /// is never shown or used; a section that cannot be cleared is logged and
    /// this process never reads it back.
    pub fn reconcile(&self) {
        let Some(backing) = &self.backing else {
            return;
        };
        let mut state = self.lock();
        // Read under the lock, so the last reconcile to run always sees the
        // latest saved setting, whatever order two saves finish in.
        if !setting_enabled(&backing.runtime_config.snapshot()) {
            state.record = None;
            // Nothing is written while the report is off and nothing is kept.
            if backing
                .store
                .section::<serde_json::Value>(USAGE_REPORT_SECTION)
                .is_some()
                && let Err(message) = backing.store.remove_section(USAGE_REPORT_SECTION)
            {
                state.stale_on_disk = true;
                tracing::warn!(
                    path = %backing.store.path().display(),
                    "{message}; the usage report is off, and its record on disk is ignored"
                );
            }
            return;
        }
        if state.record.is_some() {
            return;
        }
        let stored = (!state.stale_on_disk)
            .then(|| {
                backing
                    .store
                    .section::<UsageReportRecord>(USAGE_REPORT_SECTION)
            })
            .flatten()
            .filter(|record| is_install_id(&record.install_id));
        let record = match stored {
            Some(record) => record,
            None => {
                let created = new_install_id().and_then(|install_id| {
                    let record = UsageReportRecord {
                        install_id,
                        ..UsageReportRecord::default()
                    };
                    backing
                        .store
                        .write_section(USAGE_REPORT_SECTION, &record)
                        .map(|()| record)
                });
                match created {
                    Ok(record) => record,
                    Err(message) => {
                        tracing::warn!(
                            path = %backing.store.path().display(),
                            "{message}; the usage report has no install ID until one can be saved"
                        );
                        return;
                    }
                }
            }
        };
        state.stale_on_disk = false;
        state.record = Some(record);
    }

    /// The saved install ID, or `None` while the report is off.
    pub fn install_id(&self) -> Option<String> {
        self.lock()
            .record
            .as_ref()
            .map(|record| record.install_id.clone())
    }

    /// Note an MCP tool call at `at` from the client that sent `client_name`
    /// as its `clientInfo.name`. Only the family the name maps onto is kept.
    /// Returns `true` when the record changed and is due to be saved; the
    /// caller then calls [`Self::save`], off the request path if it likes.
    pub fn observe_mcp_call(&self, client_name: &str, at: SystemTime) -> bool {
        let today = day_of(at).to_string();
        let mut state = self.lock();
        let Some(record) = state.record.as_mut() else {
            return false;
        };
        let family = AgentFamily::of(client_name).key();
        let mut changed = false;
        if record.mcp_seen.as_ref() != Some(&today) {
            record.mcp_seen = Some(today.clone());
            changed = true;
        }
        if record.agents_seen.get(family) != Some(&today) {
            record.agents_seen.insert(family.to_string(), today);
            changed = true;
        }
        changed
    }

    /// Note a web request at `at`. Returns `true` when the record changed and
    /// is due to be saved, as [`Self::observe_mcp_call`] does.
    pub fn observe_web_request(&self, at: SystemTime) -> bool {
        let today = day_of(at).to_string();
        let mut state = self.lock();
        let Some(record) = state.record.as_mut() else {
            return false;
        };
        if record.web_seen.as_ref() == Some(&today) {
            return false;
        }
        record.web_seen = Some(today);
        true
    }

    /// Write the record to the instance state file. Never fails: a record
    /// that cannot be written is logged and kept in memory. Writes nothing
    /// once the report is off, so a save that was already on its way cannot
    /// bring a cleared section back.
    pub fn save(&self) {
        let Some(backing) = &self.backing else {
            return;
        };
        // Held across the write, so it cannot interleave with a reconcile
        // that clears the section.
        let state = self.lock();
        let Some(record) = state.record.as_ref() else {
            return;
        };
        if let Err(message) = backing.store.write_section(USAGE_REPORT_SECTION, record) {
            tracing::warn!(
                path = %backing.store.path().display(),
                "{message}; keeping the usage report's activity record in memory only"
            );
        }
    }

    /// When the collector last accepted a report with the current install
    /// ID, as saved. `None` while the report is off, and for a new ID.
    pub fn last_sent_at(&self) -> Option<String> {
        self.lock()
            .record
            .as_ref()
            .and_then(|record| record.last_sent.clone())
    }

    /// Note that the collector accepted a report carrying `install_id` at
    /// `at`, and save it. Keeps nothing when that is no longer this install's
    /// ID, so a send that finished after the report was switched off cannot
    /// bring the section back. Never fails: a time that cannot be written is
    /// logged and held in memory, which still keeps this process to one
    /// report a day.
    fn record_sent(&self, install_id: &str, at: SystemTime) {
        let Some(backing) = &self.backing else {
            return;
        };
        let mut state = self.lock();
        let Some(record) = state
            .record
            .as_mut()
            .filter(|record| record.install_id == install_id)
        else {
            return;
        };
        record.last_sent = Some(rfc3339(at));
        if let Err(message) = backing.store.write_section(USAGE_REPORT_SECTION, record) {
            tracing::warn!(
                path = %backing.store.path().display(),
                "{message}; keeping the time of the last usage report in memory only"
            );
        }
    }

    /// What the activity record says as of `today`. All `false` while the
    /// report is off.
    fn activity(&self, today: NaiveDate) -> Activity {
        let state = self.lock();
        let Some(record) = state.record.as_ref() else {
            return Activity::default();
        };
        Activity {
            mcp_active_7d: seen_within(record.mcp_seen.as_ref(), today, ACTIVE_WINDOW_DAYS),
            web_active_7d: seen_within(record.web_seen.as_ref(), today, ACTIVE_WINDOW_DAYS),
            agents: AgentFamily::ALL.map(|family| {
                seen_within(
                    record.agents_seen.get(family.key()),
                    today,
                    AGENT_WINDOW_DAYS,
                )
            }),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, SectionState> {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

/// The yes or no answers the activity record gives. `agents` follows
/// [`AgentFamily::ALL`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Activity {
    mcp_active_7d: bool,
    web_active_7d: bool,
    agents: [bool; 9],
}

/// What the build knows about itself. An input to the report, so tests do not
/// depend on how the test binary was built.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Build {
    /// The base version, such as `2.8.0`.
    pub version: String,
    /// `stable` for a release, `dev` for every other build.
    pub channel: &'static str,
    /// `docker`, `podman` or `source`.
    pub image: &'static str,
    pub os: &'static str,
    pub arch: &'static str,
}

impl Build {
    /// The running build.
    pub fn current() -> Self {
        let version = crate::config::version_string();
        let base = crate::instance_state::base_version(&version);
        Self {
            channel: crate::config::build_channel(),
            version: base.to_string(),
            image: crate::config::build_image(),
            os: std::env::consts::OS,
            arch: std::env::consts::ARCH,
        }
    }
}

/// What the install's current state says, before it is reduced to words and
/// buckets.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct InstallFacts {
    search_model: Option<SelectedModel>,
    enabled_vaults: usize,
    notes: usize,
    /// The Git mode of each enabled Vault that has a remote.
    remote_modes: Vec<VaultGitMode>,
    mcp_enabled: bool,
    mcp_writes: bool,
}

impl InstallFacts {
    /// Read off the async runtime: a registry file, one row count per
    /// enabled Vault and the model selection. Settings asks on every load, so
    /// it counts rows and never reads a Vault's notes.
    async fn gather(state: &AppState, snapshot: &ConfigSnapshot) -> Self {
        let registry = state.vault_registry.clone();
        let cache = state.startup_sqlite.clone();
        let model_setup = state.model_setup.clone();
        let mcp_enabled = setting_on(snapshot, "HATCHDOOR_MCP_ENABLED");
        let mcp_writes = setting_on(snapshot, "HATCHDOOR_MCP_WRITE_ENABLED");
        tokio::task::spawn_blocking(move || {
            let enabled: Vec<_> = match registry.load() {
                Ok(VaultRegistryState::Ready(registry)) => registry
                    .definitions()
                    .filter(|definition| definition.enabled())
                    .collect(),
                _ => Vec::new(),
            };
            Self {
                search_model: model_setup.selected().ok(),
                enabled_vaults: enabled.len(),
                // A Vault with nothing indexed yet counts no notes.
                notes: enabled
                    .iter()
                    .map(|definition| {
                        cache
                            .snapshot_note_count(definition.vault_id())
                            .unwrap_or(0)
                    })
                    .sum(),
                remote_modes: enabled
                    .iter()
                    .filter_map(|definition| remote_mode(definition.source()))
                    .collect(),
                mcp_enabled,
                mcp_writes,
            }
        })
        .await
        .unwrap_or_else(|_| Self {
            mcp_enabled,
            mcp_writes,
            ..Self::default()
        })
    }
}

/// The Git mode of a Vault that has a remote, or `None` for a plain folder
/// or local history only.
fn remote_mode(source: &VaultSource) -> Option<VaultGitMode> {
    match source {
        VaultSource::ExistingGit { mode, .. } | VaultSource::ManagedGit { mode, .. } => {
            (*mode != VaultGitMode::LocalHistory).then_some(*mode)
        }
        VaultSource::Local { .. } => None,
    }
}

fn search_model_word(model: Option<SelectedModel>) -> &'static str {
    match model {
        Some(SelectedModel::Gemma) => "gemma",
        Some(SelectedModel::Nomic) => "nomic",
        Some(SelectedModel::TermsRequired) | None => "none",
    }
}

fn vaults_bucket(count: usize) -> &'static str {
    match count {
        0 => "0",
        1 => "1",
        2..=3 => "2-3",
        _ => "4+",
    }
}

fn notes_bucket(count: usize) -> &'static str {
    match count {
        0 => "0",
        1..=99 => "1-99",
        100..=999 => "100-999",
        1_000..=9_999 => "1k-9k",
        _ => "10k+",
    }
}

/// `none` when no enabled Vault has a remote, the mode when every one that
/// has a remote uses it, and `mixed` otherwise.
fn git_sync_word(remote_modes: &[VaultGitMode]) -> &'static str {
    let word = |mode: &VaultGitMode| match mode {
        VaultGitMode::PullOnly => "pull_only",
        VaultGitMode::TwoWay => "two_way",
        VaultGitMode::LocalHistory => "none",
    };
    match remote_modes.split_first() {
        None => "none",
        Some((first, rest)) if rest.iter().all(|mode| mode == first) => word(first),
        Some(_) => "mixed",
    }
}

/// The report, in the event format of the analytics service it goes to
/// (ADR-45). Field order is the order sent and shown.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct UsageReportBody {
    #[serde(rename = "type")]
    kind: &'static str,
    payload: ReportPayload,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
struct ReportPayload {
    website: &'static str,
    hostname: &'static str,
    url: &'static str,
    name: &'static str,
    /// The install ID.
    id: String,
    data: ReportData,
}

/// Everything the report says about the install. Nothing else is sent.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
struct ReportData {
    schema: u32,
    version: String,
    channel: &'static str,
    os: &'static str,
    arch: &'static str,
    image: &'static str,
    search_model: &'static str,
    vaults: &'static str,
    notes: &'static str,
    git_sync: &'static str,
    mcp_enabled: bool,
    mcp_writes: bool,
    mcp_active_7d: bool,
    web_active_7d: bool,
    agent_claude_code: bool,
    agent_claude_desktop: bool,
    agent_codex: bool,
    agent_cursor: bool,
    agent_vscode: bool,
    agent_chatgpt: bool,
    agent_openclaw: bool,
    agent_hermes: bool,
    agent_other: bool,
}

fn compose(
    build: &Build,
    facts: &InstallFacts,
    activity: Activity,
    install_id: &str,
) -> UsageReportBody {
    let [
        agent_claude_code,
        agent_claude_desktop,
        agent_codex,
        agent_cursor,
        agent_vscode,
        agent_chatgpt,
        agent_openclaw,
        agent_hermes,
        agent_other,
    ] = activity.agents;
    UsageReportBody {
        kind: "event",
        payload: ReportPayload {
            website: COLLECTOR_WEBSITE_ID,
            hostname: "hatchdoor",
            url: "/report",
            name: "report",
            id: install_id.to_string(),
            data: ReportData {
                schema: REPORT_SCHEMA,
                version: build.version.clone(),
                channel: build.channel,
                os: build.os,
                arch: build.arch,
                image: build.image,
                search_model: search_model_word(facts.search_model),
                vaults: vaults_bucket(facts.enabled_vaults),
                notes: notes_bucket(facts.notes),
                git_sync: git_sync_word(&facts.remote_modes),
                mcp_enabled: facts.mcp_enabled,
                mcp_writes: facts.mcp_writes,
                mcp_active_7d: activity.mcp_active_7d,
                web_active_7d: activity.web_active_7d,
                agent_claude_code,
                agent_claude_desktop,
                agent_codex,
                agent_cursor,
                agent_vscode,
                agent_chatgpt,
                agent_openclaw,
                agent_hermes,
                agent_other,
            },
        },
    }
}

/// The report the next send would carry, as of `now`: `build` as given, and
/// everything else read from the install's current state. While the report is
/// off, or its ID could not be saved, the ID is [`INSTALL_ID_PLACEHOLDER`].
pub async fn current_report(
    state: &AppState,
    snapshot: &ConfigSnapshot,
    build: &Build,
    now: SystemTime,
) -> UsageReportBody {
    let facts = InstallFacts::gather(state, snapshot).await;
    let install_id =
        usable_install_id(state, snapshot).unwrap_or_else(|| INSTALL_ID_PLACEHOLDER.to_string());
    let activity = if setting_enabled(snapshot) {
        state.usage_report.activity(day_of(now))
    } else {
        Activity::default()
    };
    compose(build, &facts, activity, &install_id)
}

/// What the settings response reports about the usage report.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct UsageReportStatus {
    pub enabled: bool,
    /// The install ID while the report is on and the ID is saved.
    pub install_id: Option<String>,
    /// The exact report the next send would carry, as indented JSON text.
    pub report: String,
    /// When the collector last accepted a report from this install ID, while
    /// the report is on. RFC 3339, seconds, UTC.
    pub last_sent_at: Option<String>,
}

/// The status for a settings response under `snapshot`.
pub async fn status(state: &AppState, snapshot: &ConfigSnapshot) -> UsageReportStatus {
    let report = current_report(state, snapshot, &Build::current(), SystemTime::now()).await;
    UsageReportStatus {
        enabled: setting_enabled(snapshot),
        install_id: usable_install_id(state, snapshot),
        report: serde_json::to_string_pretty(&report).unwrap_or_default(),
        last_sent_at: setting_enabled(snapshot)
            .then(|| state.usage_report.last_sent_at())
            .flatten(),
    }
}

/// The one request a send makes. Everything the collector is told is here.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReportRequest {
    pub url: &'static str,
    pub user_agent: &'static str,
    /// The report, as JSON.
    pub body: String,
}

/// Posts one report and says whether the collector accepted it.
pub type SendReport = Arc<dyn Fn(&ReportRequest) -> Result<(), String> + Send + Sync>;

/// Decides, on each tick, whether a report is due, and sends it.
pub struct Reporter {
    state: AppState,
    send: SendReport,
    build: Build,
    /// The last send this process tried, with any ID. Nothing is tried again
    /// within [`TICK_INTERVAL`] of it.
    last_attempt: Option<SystemTime>,
    /// The last send that failed and the ID it carried. Kept in memory only:
    /// a restart tries again at its first tick.
    last_failure: Option<(String, SystemTime)>,
}

impl Reporter {
    /// A reporter for the running build, on `state`'s live setting and usage
    /// report record.
    pub fn new(state: AppState, send: SendReport) -> Self {
        Self {
            state,
            send,
            build: Build::current(),
            last_attempt: None,
            last_failure: None,
        }
    }

    /// Run one tick at `now`. Returns whether it made a request. Waits for
    /// as long as the request takes.
    pub async fn tick(&mut self, now: SystemTime) -> bool {
        let snapshot = self.state.runtime_snapshot();
        // Off, or an ID that could not be saved: nothing is sent.
        let Some(install_id) = usable_install_id(&self.state, &snapshot) else {
            return false;
        };
        let last_sent = self
            .state
            .usage_report
            .last_sent_at()
            .and_then(|sent| parse_rfc3339(&sent));
        let failed_recently = self.last_failure.as_ref().is_some_and(|(id, at)| {
            *id == install_id && within(Some(*at), now, RETRY_INTERVAL - TIMER_SLACK)
        });
        if within(self.last_attempt, now, TICK_INTERVAL - TIMER_SLACK)
            || within(last_sent, now, REPORT_INTERVAL)
            || failed_recently
        {
            return false;
        }
        let report = current_report(&self.state, &snapshot, &self.build, now).await;
        let Ok(body) = serde_json::to_string(&report) else {
            return false;
        };
        let request = ReportRequest {
            url: COLLECTOR_URL,
            user_agent: USER_AGENT,
            body,
        };
        let state = self.state.clone();
        let send = self.send.clone();
        let sending_as = install_id.clone();
        // The request blocks, and so does the write that follows it.
        let outcome = tokio::task::spawn_blocking(move || {
            // The setting may have gone off, or off and on again, while the
            // report was put together. Then this report is not sent.
            let current = usable_install_id(&state, &state.runtime_snapshot());
            if current.as_deref() != Some(sending_as.as_str()) || report.payload.id != sending_as {
                return None;
            }
            let outcome = send(&request);
            if outcome.is_ok() {
                state.usage_report.record_sent(&sending_as, now);
            }
            Some(outcome)
        })
        .await;
        let outcome = match outcome {
            Ok(Some(outcome)) => outcome,
            Ok(None) => return false,
            Err(error) => Err(error.to_string()),
        };
        self.last_attempt = Some(now);
        match outcome {
            Ok(()) => self.last_failure = None,
            Err(message) => {
                tracing::debug!(
                    "Could not send the usage report ({message}); trying again in an hour"
                );
                self.last_failure = Some((install_id, now));
            }
        }
        true
    }
}

/// Run `reporter` every [`TICK_INTERVAL`] until `shutdown` fires, on a task
/// of its own. A demo instance starts nothing and gets `None`.
pub fn spawn(
    mut reporter: Reporter,
    demo_mode: bool,
    shutdown: crate::app_state::ShutdownSignal,
) -> Option<tokio::task::JoinHandle<()>> {
    if demo_mode {
        return None;
    }
    Some(tokio::spawn(async move {
        let mut ticks = tokio::time::interval(TICK_INTERVAL);
        ticks.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            tokio::select! {
                _ = ticks.tick() => {}
                () = shutdown.wait() => return,
            }
            reporter.tick(SystemTime::now()).await;
        }
    }))
}

/// The real request: one `POST` of the report to the collector with the
/// `Hatchdoor` user-agent. A redirect is not followed, so the report goes to
/// the declared address or nowhere, and nothing in the answer is read beyond
/// whether it was a success.
pub fn collector() -> SendReport {
    Arc::new(|request| {
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .tls_config(
                ureq::tls::TlsConfig::builder()
                    .provider(ureq::tls::TlsProvider::NativeTls)
                    .build(),
            )
            .timeout_global(Some(REQUEST_TIMEOUT))
            .user_agent(request.user_agent)
            .max_redirects(0)
            .build()
            .into();
        let response = agent
            .post(request.url)
            .header("Content-Type", "application/json")
            .send(request.body.as_str())
            .map_err(|error| error.to_string())?;
        if response.status().is_success() {
            Ok(())
        } else {
            Err(format!("the collector answered {}", response.status()))
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;
    use std::sync::Arc;
    use std::time::Duration;

    use crate::runtime_config::Environment;

    struct Fixture {
        dir: tempfile::TempDir,
        runtime_config: RuntimeConfig,
        store: InstanceStateStore,
    }

    impl Fixture {
        fn new() -> Self {
            Self::in_dir(tempfile::tempdir().unwrap(), None)
        }

        /// An install whose settings and state live in `dir`, started with
        /// the setting pinned by the environment to `pinned`, if given.
        fn in_dir(dir: tempfile::TempDir, pinned: Option<&str>) -> Self {
            let environment = Environment::from_values(
                pinned.map(|value| (USAGE_REPORT_SETTING.to_string(), value.to_string())),
            );
            let runtime_config = RuntimeConfig::load(
                dir.path().join("settings.json"),
                environment,
                crate::runtime_config::live_settings_defaults(),
            )
            .unwrap();
            let store = InstanceStateStore::new(dir.path().join("state/instance.json"));
            Self {
                dir,
                runtime_config,
                store,
            }
        }

        fn handle(&self) -> UsageReport {
            self.handle_in_demo_mode(false)
        }

        fn handle_in_demo_mode(&self, demo_mode: bool) -> UsageReport {
            let handle =
                UsageReport::new(self.store.clone(), self.runtime_config.clone(), demo_mode);
            handle.reconcile();
            handle
        }

        fn set_enabled(&self, handle: &UsageReport, enabled: bool) {
            self.runtime_config
                .save([(USAGE_REPORT_SETTING.to_string(), enabled.to_string())])
                .unwrap();
            handle.reconcile();
        }

        fn stored(&self) -> Option<serde_json::Value> {
            self.store.section(USAGE_REPORT_SECTION)
        }
    }

    fn at_day(day: &str) -> SystemTime {
        let date = NaiveDate::parse_from_str(day, "%Y-%m-%d").unwrap();
        let seconds = date.and_hms_opt(12, 0, 0).unwrap().and_utc().timestamp();
        SystemTime::UNIX_EPOCH + Duration::from_secs(seconds as u64)
    }

    fn day(day: &str) -> NaiveDate {
        NaiveDate::parse_from_str(day, "%Y-%m-%d").unwrap()
    }

    fn observe_mcp(handle: &UsageReport, name: &str, day: &str) -> bool {
        let due = handle.observe_mcp_call(name, at_day(day));
        if due {
            handle.save();
        }
        due
    }

    fn observe_web(handle: &UsageReport, day: &str) -> bool {
        let due = handle.observe_web_request(at_day(day));
        if due {
            handle.save();
        }
        due
    }

    #[test]
    fn while_the_setting_is_off_nothing_is_kept_and_the_hooks_write_nothing() {
        let fixture = Fixture::new();
        let handle = fixture.handle();

        assert_eq!(handle.install_id(), None);
        assert!(!observe_mcp(&handle, "claude-code", "2026-10-07"));
        assert!(!observe_web(&handle, "2026-10-07"));
        handle.save();

        assert_eq!(fixture.stored(), None);
        assert!(!fixture.store.path().exists());
    }

    #[test]
    fn turning_it_on_creates_an_id_off_clears_the_section_and_on_again_is_a_new_id() {
        let fixture = Fixture::new();
        let handle = fixture.handle();

        fixture.set_enabled(&handle, true);
        let first = handle.install_id().expect("an ID once the report is on");
        assert!(is_install_id(&first));
        assert_eq!(first.as_bytes()[14], b'4', "a version 4 UUID: {first}");
        assert_eq!(fixture.stored().unwrap()["install_id"], first.as_str());
        observe_mcp(&handle, "claude-code", "2026-10-07");
        observe_web(&handle, "2026-10-07");

        fixture.set_enabled(&handle, false);
        assert_eq!(handle.install_id(), None);
        assert_eq!(fixture.stored(), None);

        fixture.set_enabled(&handle, true);
        let second = handle.install_id().expect("a new ID");
        assert_ne!(second, first);
        assert_eq!(
            fixture.stored().unwrap(),
            serde_json::json!({ "install_id": second }),
            "the activity record went with the old ID"
        );
    }

    #[test]
    fn the_id_survives_a_restart_and_a_save_of_another_setting() {
        let fixture = Fixture::new();
        let handle = fixture.handle();
        fixture.set_enabled(&handle, true);
        let id = handle.install_id().unwrap();
        observe_web(&handle, "2026-10-07");

        handle.reconcile();
        assert_eq!(handle.install_id().as_deref(), Some(id.as_str()));

        let restarted = fixture.handle();
        assert_eq!(restarted.install_id().as_deref(), Some(id.as_str()));
        assert!(
            !observe_web(&restarted, "2026-10-07"),
            "the day already recorded is not written again"
        );
    }

    #[test]
    fn the_environment_turns_it_on_and_off_across_a_restart() {
        let pinned_on = Fixture::in_dir(tempfile::tempdir().unwrap(), Some("true"));
        let first = pinned_on.handle().install_id().expect("on from the start");

        let pinned_off = Fixture::in_dir(pinned_on.dir, Some("false"));
        let handle = pinned_off.handle();
        assert_eq!(handle.install_id(), None);
        assert_eq!(pinned_off.stored(), None);

        let pinned_on_again = Fixture::in_dir(pinned_off.dir, Some("true"));
        let second = pinned_on_again.handle().install_id().expect("on again");
        assert_ne!(second, first);
    }

    #[test]
    fn a_demo_instance_keeps_nothing_even_with_the_variable_set() {
        let fixture = Fixture::in_dir(tempfile::tempdir().unwrap(), Some("true"));
        let handle = fixture.handle_in_demo_mode(true);

        assert_eq!(handle.install_id(), None);
        assert!(!observe_mcp(&handle, "claude-code", "2026-10-07"));
        assert!(!observe_web(&handle, "2026-10-07"));
        assert!(!fixture.store.path().exists());
    }

    #[test]
    fn an_id_that_could_not_be_saved_is_not_used() {
        let fixture = Fixture::in_dir(tempfile::tempdir().unwrap(), Some("true"));
        // A file where the state directory should be: nothing can be written.
        std::fs::write(fixture.dir.path().join("state"), b"in the way").unwrap();
        let handle = fixture.handle();

        assert_eq!(handle.install_id(), None);
        assert!(!handle.observe_web_request(at_day("2026-10-07")));
        assert_eq!(handle.activity(day("2026-10-07")), Activity::default());
    }

    #[test]
    fn a_save_already_on_its_way_cannot_bring_a_cleared_section_back() {
        let fixture = Fixture::new();
        let handle = fixture.handle();
        fixture.set_enabled(&handle, true);
        assert!(handle.observe_web_request(at_day("2026-10-07")));

        fixture.set_enabled(&handle, false);
        handle.save();

        assert_eq!(fixture.stored(), None);
    }

    #[test]
    fn each_fact_is_written_at_most_once_a_day() {
        let fixture = Fixture::new();
        let handle = fixture.handle();
        fixture.set_enabled(&handle, true);

        assert!(observe_mcp(&handle, "claude-code", "2026-10-07"));
        assert!(!observe_mcp(&handle, "claude-code", "2026-10-07"));
        assert!(observe_mcp(&handle, "cursor-vscode", "2026-10-07"));
        assert!(observe_web(&handle, "2026-10-07"));
        assert!(!observe_web(&handle, "2026-10-07"));
        assert!(observe_web(&handle, "2026-10-08"));

        let stored = fixture.stored().unwrap();
        assert_eq!(stored["mcp_seen"], "2026-10-07");
        assert_eq!(stored["web_seen"], "2026-10-08");
        assert_eq!(
            stored["agents_seen"],
            serde_json::json!({ "claude_code": "2026-10-07", "cursor": "2026-10-07" })
        );
    }

    #[test]
    fn client_names_map_onto_the_closed_list_of_families() {
        for (name, family) in [
            ("claude-code", AgentFamily::ClaudeCode),
            ("claude-ai", AgentFamily::ClaudeDesktop),
            ("Anthropic/ClaudeAI", AgentFamily::ClaudeDesktop),
            ("codex-mcp-client", AgentFamily::Codex),
            ("cursor-vscode", AgentFamily::Cursor),
            ("Visual Studio Code", AgentFamily::VsCode),
            ("Visual Studio Code - Insiders", AgentFamily::VsCode),
            ("openai-mcp", AgentFamily::ChatGpt),
            ("ChatGPT", AgentFamily::ChatGpt),
            ("openclaw", AgentFamily::OpenClaw),
            ("hermes-agent", AgentFamily::Hermes),
            ("  Claude-Code ", AgentFamily::ClaudeCode),
            ("Totally Secret Agent 9000", AgentFamily::Other),
            ("", AgentFamily::Other),
        ] {
            assert_eq!(AgentFamily::of(name), family, "{name:?}");
        }
    }

    #[test]
    fn a_raw_agent_name_never_reaches_the_record_or_the_report() {
        let fixture = Fixture::new();
        let handle = fixture.handle();
        fixture.set_enabled(&handle, true);

        observe_mcp(&handle, "Totally Secret Agent 9000", "2026-10-07");

        let stored = fixture.stored().unwrap();
        assert_eq!(
            stored["agents_seen"],
            serde_json::json!({ "other": "2026-10-07" })
        );
        let report = compose(
            &test_build(),
            &InstallFacts::default(),
            handle.activity(day("2026-10-07")),
            &handle.install_id().unwrap(),
        );
        for text in [
            stored.to_string(),
            serde_json::to_string(&report).unwrap(),
            std::fs::read_to_string(fixture.store.path()).unwrap(),
        ] {
            assert!(!text.contains("Secret"), "{text}");
        }
    }

    #[test]
    fn activity_counts_seven_days_and_agents_thirty() {
        let fixture = Fixture::new();
        let handle = fixture.handle();
        fixture.set_enabled(&handle, true);
        observe_mcp(&handle, "codex-mcp-client", "2026-09-01");
        observe_web(&handle, "2026-09-01");

        let on = |today: &str| handle.activity(day(today));
        let codex = AgentFamily::ALL
            .iter()
            .position(|family| *family == AgentFamily::Codex)
            .unwrap();

        assert!(on("2026-09-01").mcp_active_7d);
        assert!(on("2026-09-07").mcp_active_7d && on("2026-09-07").web_active_7d);
        assert!(!on("2026-09-08").mcp_active_7d && !on("2026-09-08").web_active_7d);
        assert!(on("2026-09-30").agents[codex]);
        assert!(!on("2026-10-01").agents[codex]);
        // A clock that moved back does not make the past active.
        assert!(!on("2026-08-31").mcp_active_7d);
        assert_eq!(
            on("2026-09-30").agents.iter().filter(|seen| **seen).count(),
            1
        );
    }

    fn test_build() -> Build {
        Build {
            version: "2.8.0".into(),
            channel: "stable",
            image: "docker",
            os: "linux",
            arch: "x86_64",
        }
    }

    #[test]
    fn a_build_that_was_not_told_it_is_a_release_reports_dev() {
        // This test binary is built with no `VERSION` build argument.
        let build = Build::current();
        assert_eq!(build.channel, "dev");
        assert_eq!(build.version, env!("CARGO_PKG_VERSION"));
    }

    #[test]
    fn the_report_for_a_known_state_is_exactly_this() {
        let facts = InstallFacts {
            search_model: Some(SelectedModel::Gemma),
            enabled_vaults: 2,
            notes: 1_234,
            remote_modes: vec![VaultGitMode::TwoWay],
            mcp_enabled: true,
            mcp_writes: false,
        };
        let activity = Activity {
            mcp_active_7d: true,
            web_active_7d: false,
            agents: [true, false, false, false, false, false, false, false, true],
        };

        let report = compose(
            &test_build(),
            &facts,
            activity,
            "0b1c2d3e-4f50-4a61-8b72-93a4b5c6d7e8",
        );

        assert_eq!(
            serde_json::to_string(&report).unwrap(),
            concat!(
                r#"{"type":"event","payload":{"website":"7583bdb0-bd67-4203-8fc3-2a017c5a611d","#,
                r#""hostname":"hatchdoor","url":"/report","name":"report","#,
                r#""id":"0b1c2d3e-4f50-4a61-8b72-93a4b5c6d7e8","data":{"schema":1,"#,
                r#""version":"2.8.0","channel":"stable","os":"linux","arch":"x86_64","#,
                r#""image":"docker","search_model":"gemma","vaults":"2-3","notes":"1k-9k","#,
                r#""git_sync":"two_way","mcp_enabled":true,"mcp_writes":false,"#,
                r#""mcp_active_7d":true,"web_active_7d":false,"agent_claude_code":true,"#,
                r#""agent_claude_desktop":false,"agent_codex":false,"agent_cursor":false,"#,
                r#""agent_vscode":false,"agent_chatgpt":false,"agent_openclaw":false,"#,
                r#""agent_hermes":false,"agent_other":true}}}"#,
            )
        );
    }

    #[test]
    fn every_bucket_has_its_edges() {
        for (count, bucket) in [
            (0, "0"),
            (1, "1"),
            (2, "2-3"),
            (3, "2-3"),
            (4, "4+"),
            (40, "4+"),
        ] {
            assert_eq!(vaults_bucket(count), bucket, "{count} Vaults");
        }
        for (count, bucket) in [
            (0, "0"),
            (1, "1-99"),
            (99, "1-99"),
            (100, "100-999"),
            (999, "100-999"),
            (1_000, "1k-9k"),
            (9_999, "1k-9k"),
            (10_000, "10k+"),
            (2_000_000, "10k+"),
        ] {
            assert_eq!(notes_bucket(count), bucket, "{count} notes");
        }
    }

    #[test]
    fn git_sync_is_none_one_mode_or_mixed() {
        use VaultGitMode::{PullOnly, TwoWay};
        assert_eq!(git_sync_word(&[]), "none");
        assert_eq!(git_sync_word(&[PullOnly]), "pull_only");
        assert_eq!(git_sync_word(&[TwoWay, TwoWay]), "two_way");
        assert_eq!(git_sync_word(&[TwoWay, PullOnly]), "mixed");
    }

    #[test]
    fn only_a_vault_with_a_remote_counts_towards_git_sync() {
        let git = |mode| VaultSource::ExistingGit {
            repository_path: "/vault".into(),
            repository_url: None,
            branch: None,
            vault_subdirectory: None,
            mode,
            poll_interval_secs: 60,
        };
        assert_eq!(
            remote_mode(&VaultSource::Local {
                path: "/vault".into()
            }),
            None
        );
        assert_eq!(remote_mode(&git(VaultGitMode::LocalHistory)), None);
        assert_eq!(
            remote_mode(&git(VaultGitMode::PullOnly)),
            Some(VaultGitMode::PullOnly)
        );
    }

    #[test]
    fn the_search_model_is_one_of_three_words() {
        assert_eq!(search_model_word(Some(SelectedModel::Gemma)), "gemma");
        assert_eq!(search_model_word(Some(SelectedModel::Nomic)), "nomic");
        assert_eq!(
            search_model_word(Some(SelectedModel::TermsRequired)),
            "none"
        );
        assert_eq!(search_model_word(None), "none");
    }

    use std::sync::atomic::Ordering;

    /// An install whose state lives in a temp dir, and a sender that keeps
    /// every request it is handed and answers what the test last told it to.
    struct SendFixture {
        state: AppState,
        store: InstanceStateStore,
        requests: Arc<Mutex<Vec<ReportRequest>>>,
        failing: Arc<std::sync::atomic::AtomicBool>,
        _worker: crate::vault_work::VaultWorkWorker,
        dir: tempfile::TempDir,
    }

    impl SendFixture {
        fn new() -> Self {
            Self::with_state_at("state/instance.json")
        }

        fn with_state_at(relative: &str) -> Self {
            let (mut state, worker, dir) = crate::vault_management::test_support::test_state();
            let store = InstanceStateStore::new(dir.path().join(relative));
            state.usage_report = Arc::new(UsageReport::new(
                store.clone(),
                state.runtime_config.clone(),
                false,
            ));
            state.usage_report.reconcile();
            Self {
                state,
                store,
                requests: Arc::default(),
                failing: Arc::default(),
                _worker: worker,
                dir,
            }
        }

        fn reporter(&self) -> Reporter {
            let requests = self.requests.clone();
            let failing = self.failing.clone();
            Reporter::new(
                self.state.clone(),
                Arc::new(move |request: &ReportRequest| {
                    requests.lock().unwrap().push(request.clone());
                    if failing.load(Ordering::SeqCst) {
                        Err("connection refused".to_string())
                    } else {
                        Ok(())
                    }
                }),
            )
        }

        fn set_enabled(&self, enabled: bool) {
            self.state
                .runtime_config
                .save([(USAGE_REPORT_SETTING.to_string(), enabled.to_string())])
                .unwrap();
            self.state.usage_report.reconcile();
        }

        fn fail_sends(&self, failing: bool) {
            self.failing.store(failing, Ordering::SeqCst);
        }

        fn requests(&self) -> Vec<ReportRequest> {
            self.requests.lock().unwrap().clone()
        }

        fn sent(&self) -> usize {
            self.requests().len()
        }
    }

    fn at(seconds: u64) -> SystemTime {
        SystemTime::UNIX_EPOCH + Duration::from_secs(1_800_000_000 + seconds)
    }

    const MINUTE: u64 = 60;
    const HOUR: u64 = 60 * MINUTE;
    const DAY: u64 = 24 * HOUR;

    #[tokio::test]
    async fn while_the_setting_is_off_no_request_is_ever_made() {
        let fixture = SendFixture::new();
        let mut reporter = fixture.reporter();
        for tick in 0..5 {
            assert!(!reporter.tick(at(tick * DAY)).await);
        }
        assert_eq!(fixture.sent(), 0);
        assert!(!fixture.store.path().exists());
    }

    #[tokio::test]
    async fn switching_on_sends_one_report_whose_body_is_the_settings_preview() {
        let fixture = SendFixture::new();
        let mut reporter = fixture.reporter();
        assert!(!reporter.tick(at(0)).await);

        fixture.set_enabled(true);
        let shown = status(&fixture.state, &fixture.state.runtime_snapshot()).await;
        assert!(reporter.tick(at(MINUTE)).await);

        let requests = fixture.requests();
        assert_eq!(requests.len(), 1);
        assert_eq!(
            requests[0].url,
            "https://telemetry-hatchdoor.battercloud.cc/v1/report"
        );
        assert_eq!(requests[0].user_agent, "Hatchdoor");
        let json = |text: &str| serde_json::from_str::<serde_json::Value>(text).unwrap();
        assert_eq!(json(&requests[0].body), json(&shown.report));
        assert_eq!(
            json(&requests[0].body)["payload"]["id"],
            shown
                .install_id
                .expect("an ID once the report is on")
                .as_str()
        );
    }

    #[tokio::test]
    async fn a_second_report_waits_a_day_after_a_success_even_across_a_restart() {
        let fixture = SendFixture::new();
        fixture.set_enabled(true);
        let mut reporter = fixture.reporter();
        assert!(reporter.tick(at(0)).await);
        assert_eq!(
            fixture
                .store
                .section::<serde_json::Value>(USAGE_REPORT_SECTION)
                .unwrap()["last_sent"],
            "2027-01-15T08:00:00Z"
        );
        assert!(!reporter.tick(at(HOUR)).await);
        assert!(!reporter.tick(at(DAY - 1)).await);

        // A restart: a new handle on the same file, and a new job.
        let restarted = Arc::new(UsageReport::new(
            fixture.store.clone(),
            fixture.state.runtime_config.clone(),
            false,
        ));
        restarted.reconcile();
        let mut state = fixture.state.clone();
        state.usage_report = restarted;
        let requests = fixture.requests.clone();
        let mut after_restart = Reporter::new(
            state,
            Arc::new(move |request: &ReportRequest| {
                requests.lock().unwrap().push(request.clone());
                Ok(())
            }),
        );
        assert!(!after_restart.tick(at(2 * HOUR)).await);
        assert!(!after_restart.tick(at(DAY - 1)).await);
        assert_eq!(fixture.sent(), 1);
        assert!(after_restart.tick(at(DAY)).await);
        assert_eq!(fixture.sent(), 2);
    }

    #[derive(Clone, Default)]
    struct CapturedLogs(Arc<Mutex<Vec<u8>>>);

    impl CapturedLogs {
        fn dispatch(&self) -> tracing::Dispatch {
            let sink = self.clone();
            tracing::Dispatch::new(
                tracing_subscriber::fmt()
                    .with_max_level(tracing::Level::DEBUG)
                    .with_target(false)
                    .with_ansi(false)
                    .compact()
                    .with_writer(move || sink.clone())
                    .finish(),
            )
        }

        fn text(&self) -> String {
            String::from_utf8(self.0.lock().unwrap().clone()).unwrap()
        }
    }

    impl std::io::Write for CapturedLogs {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(buf);
            Ok(buf.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn a_failed_send_is_logged_at_debug_retried_an_hour_later_and_leaves_nothing_behind() {
        let fixture = SendFixture::new();
        fixture.set_enabled(true);
        let before = std::fs::read_to_string(fixture.store.path()).unwrap();
        fixture.fail_sends(true);
        let mut reporter = fixture.reporter();
        let logs = CapturedLogs::default();
        let _guard = tracing::dispatcher::set_default(&logs.dispatch());

        assert!(reporter.tick(at(0)).await);
        assert!(!reporter.tick(at(MINUTE)).await);
        assert!(!reporter.tick(at(HOUR - 2)).await);
        assert_eq!(fixture.sent(), 1);
        let logged = logs.text();
        assert!(logged.contains("DEBUG"), "{logged}");
        assert!(logged.contains("connection refused"), "{logged}");
        assert!(
            !logged.contains("WARN") && !logged.contains("INFO"),
            "{logged}"
        );
        // Nothing queued: the state file is as it was, and nothing sits beside it.
        assert_eq!(
            std::fs::read_to_string(fixture.store.path()).unwrap(),
            before
        );
        let beside: Vec<_> = std::fs::read_dir(fixture.store.path().parent().unwrap())
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        assert_eq!(beside, ["instance.json"]);

        assert!(reporter.tick(at(HOUR)).await);
        fixture.fail_sends(false);
        assert!(!reporter.tick(at(HOUR + MINUTE)).await);
        assert!(reporter.tick(at(2 * HOUR)).await);
        assert_eq!(fixture.sent(), 3);
        // The report that got through describes that moment: one body, sent once.
        assert!(!reporter.tick(at(3 * HOUR)).await);
    }

    #[tokio::test]
    async fn switching_off_stops_the_job_with_no_further_request() {
        let fixture = SendFixture::new();
        fixture.set_enabled(true);
        let mut reporter = fixture.reporter();
        assert!(reporter.tick(at(0)).await);

        fixture.set_enabled(false);
        for day in 1..5 {
            assert!(!reporter.tick(at(day * DAY)).await);
        }
        assert_eq!(fixture.sent(), 1);
        assert_eq!(
            fixture
                .store
                .section::<serde_json::Value>(USAGE_REPORT_SECTION),
            None
        );
    }

    #[tokio::test]
    async fn off_and_on_again_is_a_new_id_and_a_first_report_within_the_minute() {
        let fixture = SendFixture::new();
        fixture.set_enabled(true);
        let mut reporter = fixture.reporter();
        assert!(reporter.tick(at(0)).await);

        fixture.set_enabled(false);
        fixture.set_enabled(true);
        // Only the one-attempt-a-minute floor applies across the switch.
        assert!(!reporter.tick(at(MINUTE - 2)).await);
        assert!(reporter.tick(at(MINUTE)).await);
        assert!(!reporter.tick(at(2 * MINUTE)).await);

        let ids: Vec<_> = fixture
            .requests()
            .iter()
            .map(|request| {
                serde_json::from_str::<serde_json::Value>(&request.body).unwrap()["payload"]["id"]
                    .as_str()
                    .unwrap()
                    .to_string()
            })
            .collect();
        assert_eq!(ids.len(), 2);
        assert_ne!(ids[0], ids[1]);
    }

    #[tokio::test]
    async fn a_tick_the_timer_woke_a_moment_early_is_not_skipped() {
        let a_moment = Duration::from_millis(3);
        let fixture = SendFixture::new();
        fixture.set_enabled(true);
        fixture.fail_sends(true);
        let mut reporter = fixture.reporter();
        assert!(reporter.tick(at(0)).await);
        // The retry's tick, an hour on.
        assert!(reporter.tick(at(HOUR) - a_moment).await);

        // And the first tick after switching off and on again.
        fixture.set_enabled(false);
        fixture.set_enabled(true);
        assert!(reporter.tick(at(HOUR + MINUTE) - a_moment - a_moment).await);
        assert_eq!(fixture.sent(), 3);
    }

    /// The setting goes off after the tick read it and before the request:
    /// the report that was being put together is not sent.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_report_being_put_together_when_the_setting_goes_off_is_not_sent() {
        let fixture = SendFixture::new();
        fixture.set_enabled(true);
        let mut reporter = fixture.reporter();

        // Hold the record, so the tick stops just after it read the setting.
        let held = fixture.state.usage_report.lock();
        let tick = tokio::spawn(async move { reporter.tick(at(0)).await });
        std::thread::sleep(Duration::from_millis(100));
        fixture
            .state
            .runtime_config
            .save([(USAGE_REPORT_SETTING.to_string(), "false".to_string())])
            .unwrap();
        drop(held);

        assert!(!tick.await.unwrap());
        assert_eq!(fixture.sent(), 0);
    }

    #[tokio::test]
    async fn a_failure_on_the_old_id_does_not_hold_the_new_ids_first_report_back() {
        let fixture = SendFixture::new();
        fixture.set_enabled(true);
        fixture.fail_sends(true);
        let mut reporter = fixture.reporter();
        assert!(reporter.tick(at(0)).await);

        fixture.set_enabled(false);
        fixture.set_enabled(true);
        fixture.fail_sends(false);
        assert!(reporter.tick(at(MINUTE)).await);
    }

    #[tokio::test]
    async fn nothing_is_sent_with_an_id_that_could_not_be_saved() {
        let fixture = SendFixture::new();
        // A file where the state directory should be: nothing can be written.
        std::fs::write(fixture.dir.path().join("state"), b"in the way").unwrap();
        fixture.set_enabled(true);
        let mut reporter = fixture.reporter();
        for minute in 0..5 {
            assert!(!reporter.tick(at(minute * MINUTE)).await);
        }
        assert_eq!(fixture.sent(), 0);
    }

    #[tokio::test]
    async fn a_state_file_that_stops_taking_writes_still_gets_one_report_a_day() {
        let fixture = SendFixture::new();
        fixture.set_enabled(true);
        // The ID is saved; from here on every write fails.
        let file = fixture.store.path().to_path_buf();
        std::fs::remove_file(&file).unwrap();
        std::fs::create_dir(&file).unwrap();
        let mut reporter = fixture.reporter();

        assert!(reporter.tick(at(0)).await);
        for minute in 1..10 {
            assert!(!reporter.tick(at(minute * MINUTE)).await);
        }
        assert!(!reporter.tick(at(DAY - 1)).await);
        assert_eq!(fixture.sent(), 1);
        assert!(reporter.tick(at(DAY)).await);
    }

    #[tokio::test]
    async fn a_last_report_dated_in_the_future_does_not_hold_reports_off() {
        let fixture = SendFixture::new();
        fixture.set_enabled(true);
        let mut reporter = fixture.reporter();
        assert!(reporter.tick(at(30 * DAY)).await);
        // The clock moves back a month.
        assert!(reporter.tick(at(0)).await);
    }

    #[tokio::test]
    async fn settings_shows_the_time_of_the_last_report_only_while_it_is_on() {
        let fixture = SendFixture::new();
        let shown = |fixture: &SendFixture| {
            let state = fixture.state.clone();
            async move { status(&state, &state.runtime_snapshot()).await.last_sent_at }
        };
        assert_eq!(shown(&fixture).await, None);

        fixture.set_enabled(true);
        assert_eq!(shown(&fixture).await, None, "on, and nothing sent yet");
        assert!(fixture.reporter().tick(at(0)).await);
        assert_eq!(
            shown(&fixture).await.as_deref(),
            Some("2027-01-15T08:00:00Z")
        );

        fixture.set_enabled(false);
        assert_eq!(shown(&fixture).await, None);
        fixture.set_enabled(true);
        assert_eq!(shown(&fixture).await, None, "a new ID has sent nothing");
    }

    #[tokio::test(start_paused = true)]
    async fn a_demo_instance_never_starts_the_job() {
        let fixture = SendFixture::new();
        fixture.set_enabled(true);

        let task = spawn(fixture.reporter(), true, fixture.state.shutdown.clone());

        assert!(task.is_none());
        tokio::time::sleep(Duration::from_secs(3 * DAY)).await;
        assert_eq!(fixture.sent(), 0);
    }

    #[tokio::test(start_paused = true)]
    async fn the_started_job_sends_within_a_minute_of_switching_on_and_stops_on_shutdown() {
        let fixture = SendFixture::new();
        let task = spawn(fixture.reporter(), false, fixture.state.shutdown.clone())
            .expect("started outside demo mode");
        tokio::time::sleep(Duration::from_secs(5 * MINUTE)).await;
        assert_eq!(fixture.sent(), 0);

        // Half a minute into a tick: the next one is due within the minute.
        tokio::time::sleep(Duration::from_secs(30)).await;
        fixture.set_enabled(true);
        tokio::time::sleep(TICK_INTERVAL).await;
        // The clock stands still while this task keeps the runtime busy, so
        // only that one tick can have sent it.
        let waited = std::time::Instant::now();
        while fixture.sent() == 0 {
            assert!(
                waited.elapsed() < Duration::from_secs(30),
                "no report within a minute of switching on"
            );
            tokio::task::yield_now().await;
        }
        assert_eq!(fixture.sent(), 1);

        fixture.state.shutdown.trigger();
        task.await.unwrap();
    }

    /// The field names in the manual page's tables: every table row whose
    /// first cell is one name in code formatting.
    fn documented_fields(markdown: &str) -> BTreeSet<String> {
        markdown
            .lines()
            .filter_map(|line| line.trim().strip_prefix('|'))
            .filter_map(|row| row.split('|').next())
            .filter_map(|cell| cell.trim().strip_prefix('`')?.strip_suffix('`'))
            .map(str::to_string)
            .collect()
    }

    /// The keys of the report's `data`, plus the install ID's own key.
    fn reported_fields() -> BTreeSet<String> {
        let report = serde_json::to_value(compose(
            &test_build(),
            &InstallFacts::default(),
            Activity::default(),
            INSTALL_ID_PLACEHOLDER,
        ))
        .unwrap();
        let payload = report["payload"].as_object().unwrap();
        assert!(payload.contains_key("id"));
        payload["data"]
            .as_object()
            .unwrap()
            .keys()
            .cloned()
            .chain(["id".to_string()])
            .collect()
    }

    #[test]
    fn the_manual_page_documents_exactly_the_fields_the_report_carries() {
        let page = crate::docs_bundle::page("reference/usage-report-reference")
            .expect("the usage report reference is bundled");
        assert_eq!(documented_fields(&page.markdown), reported_fields());
    }

    #[test]
    fn a_field_missing_from_either_side_is_noticed() {
        let reported = reported_fields();
        let table = |fields: &BTreeSet<String>| {
            fields
                .iter()
                .map(|field| format!("| `{field}` | What it means. |\n"))
                .collect::<String>()
        };
        let complete = format!("| Field | Meaning |\n| --- | --- |\n{}", table(&reported));
        assert_eq!(documented_fields(&complete), reported);

        let mut without_one = reported.clone();
        without_one.remove("git_sync");
        assert_ne!(documented_fields(&table(&without_one)), reported);

        let with_one_more = format!("{complete}| `hostname_of_the_machine` | No. |\n");
        assert_ne!(documented_fields(&with_one_more), reported);
    }
}
