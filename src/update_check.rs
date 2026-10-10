//! The opt-in check for a newer Hatchdoor release (ADR-39).
//!
//! While `HATCHDOOR_UPDATE_CHECK_ENABLED` is on, a background task asks
//! GitHub's public latest-release API for this repository once a day and keeps
//! the answer in the instance state file, under its own `update_check`
//! section. The settings response turns that record into the status the
//! update banner reads. Nothing else uses it: Hatchdoor never downloads or
//! installs anything.
//!
//! The setting is read from the live configuration on every tick, so turning
//! it off stops the check and turning it on runs one within a minute, both
//! without a restart. A demo instance never starts the task at all.
//!
//! The request itself sits behind [`FetchLatest`], so tests replace it and
//! never reach the network.

use std::sync::Arc;
use std::time::{Duration, SystemTime};

use serde::{Deserialize, Serialize};

use crate::instance_state::InstanceStateStore;
use crate::runtime_config::{ConfigSnapshot, RuntimeConfig, is_truthy};

/// The live setting that turns the check on. Off by default (ADR-39).
pub const UPDATE_CHECK_SETTING: &str = "HATCHDOOR_UPDATE_CHECK_ENABLED";

/// The instance state section this module owns.
const UPDATE_CHECK_SECTION: &str = "update_check";

/// GitHub's public latest-release endpoint for this repository. It never
/// returns a draft or a prerelease.
const LATEST_RELEASE_URL: &str =
    "https://api.github.com/repos/BatterWorks/Hatchdoor/releases/latest";

/// The release page a version's link is built on. Built here from the
/// version rather than taken from the response, so the banner only ever links
/// to this repository.
const RELEASE_PAGE_BASE: &str = "https://github.com/BatterWorks/Hatchdoor/releases/tag/v";

/// Sent without a version, so the request says nothing about the instance
/// beyond what any web request carries (ADR-39).
const USER_AGENT: &str = "Hatchdoor";

/// How long the whole request may take before it counts as a failure.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(15);

/// The most of a response read. A release description can be long; anything
/// near this is not the answer this check expects.
const RESPONSE_LIMIT_BYTES: u64 = 1024 * 1024;

/// How long one check's answer stands, whether it succeeded or not.
pub const CHECK_INTERVAL: Duration = Duration::from_secs(24 * 60 * 60);

/// How often the task wakes to read the setting. Bounds how soon a check runs
/// after the setting is turned on.
pub const TICK_INTERVAL: Duration = Duration::from_secs(60);

/// Asks for the latest release and returns its tag, such as `v2.9.0`.
pub type FetchLatest = Arc<dyn Fn() -> Result<String, String> + Send + Sync>;

/// A release newer checks found, as the banner needs it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LatestRelease {
    /// A plain version such as `2.9.0`.
    pub version: String,
    /// The release page on GitHub.
    pub release_url: String,
}

/// The `update_check` section of the instance state file.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct UpdateCheckRecord {
    /// When the last check ran, successful or not. RFC 3339, seconds, UTC.
    pub checked_at: String,
    /// The latest release the last check found, or `None` when it failed:
    /// a failed check shows no banner (#425).
    pub latest: Option<LatestRelease>,
}

/// What the settings response reports about the check.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct UpdateCheckStatus {
    pub enabled: bool,
    pub checked_at: Option<String>,
    /// The release the banner offers, or `None` when the check is off, has
    /// not found anything, or found nothing newer than the running version.
    pub update_available: Option<LatestRelease>,
}

/// The status for a settings response: the setting as `snapshot` resolves
/// it, and the stored record compared with `running`.
pub fn status(
    snapshot: &ConfigSnapshot,
    store: &InstanceStateStore,
    running: &str,
) -> UpdateCheckStatus {
    let enabled = setting_enabled(snapshot);
    let record = store.section::<UpdateCheckRecord>(UPDATE_CHECK_SECTION);
    let update_available = record
        .as_ref()
        .and_then(|record| record.latest.clone())
        .filter(|latest| enabled && is_newer(&latest.version, running));
    UpdateCheckStatus {
        enabled,
        checked_at: record.map(|record| record.checked_at),
        update_available,
    }
}

fn setting_enabled(snapshot: &ConfigSnapshot) -> bool {
    snapshot
        .setting(UPDATE_CHECK_SETTING)
        .is_some_and(|setting| is_truthy(&setting.value))
}

/// `2.9.0` from a tag such as `v2.9.0` or `2.9.0`, and `None` for anything
/// that is not three plain numbers.
fn release_version(tag: &str) -> Option<String> {
    let version = tag.trim().strip_prefix('v').unwrap_or(tag.trim());
    parse_version(version).map(|(major, minor, patch)| format!("{major}.{minor}.{patch}"))
}

fn parse_version(version: &str) -> Option<(u64, u64, u64)> {
    let mut parts = version.split('.');
    let mut next = || -> Option<u64> {
        let part = parts.next()?;
        if part.is_empty() || !part.bytes().all(|byte| byte.is_ascii_digit()) {
            return None;
        }
        part.parse().ok()
    };
    let parsed = (next()?, next()?, next()?);
    parts.next().is_none().then_some(parsed)
}

/// Whether `latest` is a later release than `running`. A development build
/// such as `2.8.0 (dev abc123)` compares as its base version, and anything
/// either side fails to parse counts as not newer, so a surprise never shows
/// a banner.
fn is_newer(latest: &str, running: &str) -> bool {
    let running = crate::instance_state::base_version(running);
    match (parse_version(latest), parse_version(running)) {
        (Some(latest), Some(running)) => latest > running,
        _ => false,
    }
}

fn rfc3339(at: SystemTime) -> String {
    chrono::DateTime::<chrono::Utc>::from(at).to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

fn parse_rfc3339(value: &str) -> Option<SystemTime> {
    chrono::DateTime::parse_from_rfc3339(value)
        .ok()
        .map(SystemTime::from)
}

/// Whether an attempt at `last` is less than `interval` before `now`. A time
/// in the future, from a clock that moved back, is not: otherwise it could
/// hold the next check off for as long as the clock was wrong.
fn within(last: Option<SystemTime>, now: SystemTime, interval: Duration) -> bool {
    last.is_some_and(|last| {
        now.duration_since(last)
            .is_ok_and(|elapsed| elapsed < interval)
    })
}

/// Decides, on each tick, whether a check is due, and runs it.
pub struct UpdateChecker {
    runtime_config: RuntimeConfig,
    store: InstanceStateStore,
    fetch: FetchLatest,
    /// The setting as the last tick saw it, so a switch from off to on runs
    /// a check straight away. `None` before the first tick: a restart with
    /// the setting already on waits out the day like any other tick.
    was_enabled: Option<bool>,
    /// The last check this process ran. Keeps a state directory that cannot
    /// be written from turning one check a day into one a minute.
    last_attempt: Option<SystemTime>,
}

impl UpdateChecker {
    pub fn new(
        runtime_config: RuntimeConfig,
        store: InstanceStateStore,
        fetch: FetchLatest,
    ) -> Self {
        Self {
            runtime_config,
            store,
            fetch,
            was_enabled: None,
            last_attempt: None,
        }
    }

    /// Run one tick at `now`. Returns whether it asked for the latest
    /// release. Blocks for as long as the request takes.
    pub fn tick(&mut self, now: SystemTime) -> bool {
        let enabled = setting_enabled(&self.runtime_config.snapshot());
        let turned_on = self.was_enabled == Some(false) && enabled;
        self.was_enabled = Some(enabled);
        if !enabled {
            return false;
        }
        let stored_attempt = self
            .store
            .section::<UpdateCheckRecord>(UPDATE_CHECK_SECTION)
            .and_then(|record| parse_rfc3339(&record.checked_at));
        // Switching the setting on asks at once, as the operator expects, so
        // only the one-a-minute floor applies then; otherwise a check stands
        // for a day, across restarts.
        let due = if turned_on {
            !within(self.last_attempt, now, TICK_INTERVAL)
        } else {
            !within(self.last_attempt, now, CHECK_INTERVAL)
                && !within(stored_attempt, now, CHECK_INTERVAL)
        };
        if !due {
            return false;
        }
        self.last_attempt = Some(now);
        let latest = match (self.fetch)() {
            Ok(tag) => match release_version(&tag) {
                Some(version) => Some(LatestRelease {
                    release_url: format!("{RELEASE_PAGE_BASE}{version}"),
                    version,
                }),
                None => {
                    tracing::info!(
                        tag = %tag,
                        "The latest Hatchdoor release has a tag this version cannot read; trying again tomorrow"
                    );
                    None
                }
            },
            Err(message) => {
                tracing::info!(
                    "Could not check for a newer Hatchdoor release ({message}); trying again tomorrow"
                );
                None
            }
        };
        let record = UpdateCheckRecord {
            checked_at: rfc3339(now),
            latest,
        };
        if let Err(message) = self.store.write_section(UPDATE_CHECK_SECTION, &record) {
            tracing::warn!(
                path = %self.store.path().display(),
                "{message}; the update check result is not kept"
            );
        }
        true
    }
}

/// Run `checker` every [`TICK_INTERVAL`] until `shutdown` fires. Each tick
/// runs on the blocking pool, since the request blocks.
pub fn spawn(
    checker: UpdateChecker,
    shutdown: crate::app_state::ShutdownSignal,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let checker = Arc::new(std::sync::Mutex::new(checker));
        let mut ticks = tokio::time::interval(TICK_INTERVAL);
        ticks.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            tokio::select! {
                _ = ticks.tick() => {}
                () = shutdown.wait() => return,
            }
            let checker = checker.clone();
            let tick = tokio::task::spawn_blocking(move || {
                checker
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .tick(SystemTime::now());
            });
            if let Err(error) = tick.await {
                tracing::warn!("The update check tick failed: {error}");
            }
        }
    })
}

#[derive(Deserialize)]
struct GithubRelease {
    tag_name: String,
}

/// The real request: one `GET` to GitHub's latest-release API with the
/// `Hatchdoor` user-agent and nothing else of the instance's.
pub fn github_latest_release() -> FetchLatest {
    Arc::new(|| {
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .tls_config(
                ureq::tls::TlsConfig::builder()
                    .provider(ureq::tls::TlsProvider::NativeTls)
                    .build(),
            )
            .timeout_global(Some(REQUEST_TIMEOUT))
            .user_agent(USER_AGENT)
            .build()
            .into();
        let mut response = agent
            .get(LATEST_RELEASE_URL)
            .header("Accept", "application/vnd.github+json")
            .call()
            .map_err(|error| error.to_string())?;
        let release: GithubRelease = response
            .body_mut()
            .with_config()
            .limit(RESPONSE_LIMIT_BYTES)
            .read_json()
            .map_err(|error| error.to_string())?;
        Ok(release.tag_name)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use crate::runtime_config::Environment;

    struct Fixture {
        _dir: tempfile::TempDir,
        runtime_config: RuntimeConfig,
        store: InstanceStateStore,
        calls: Arc<AtomicUsize>,
    }

    impl Fixture {
        fn new(enabled_in_env: Option<&str>) -> Self {
            let dir = tempfile::tempdir().unwrap();
            let environment = Environment::from_values(
                enabled_in_env.map(|value| (UPDATE_CHECK_SETTING.to_string(), value.to_string())),
            );
            let runtime_config = RuntimeConfig::load(
                dir.path().join("settings.json"),
                environment,
                crate::runtime_config::live_settings_defaults(),
            )
            .unwrap();
            let store = InstanceStateStore::new(dir.path().join("instance.json"));
            Self {
                _dir: dir,
                runtime_config,
                store,
                calls: Arc::new(AtomicUsize::new(0)),
            }
        }

        fn checker(&self, answer: Result<&str, &str>) -> UpdateChecker {
            let calls = self.calls.clone();
            let answer = answer.map(str::to_string).map_err(str::to_string);
            UpdateChecker::new(
                self.runtime_config.clone(),
                self.store.clone(),
                Arc::new(move || {
                    calls.fetch_add(1, Ordering::SeqCst);
                    answer.clone()
                }),
            )
        }

        fn set_enabled(&self, enabled: bool) {
            self.runtime_config
                .save(BTreeMap::from([(
                    UPDATE_CHECK_SETTING.to_string(),
                    enabled.to_string(),
                )]))
                .unwrap();
        }

        fn calls(&self) -> usize {
            self.calls.load(Ordering::SeqCst)
        }

        fn status(&self, running: &str) -> UpdateCheckStatus {
            status(&self.runtime_config.snapshot(), &self.store, running)
        }
    }

    fn at(seconds: u64) -> SystemTime {
        SystemTime::UNIX_EPOCH + Duration::from_secs(1_800_000_000 + seconds)
    }

    const HOUR: u64 = 60 * 60;
    const DAY: u64 = 24 * HOUR;

    #[test]
    fn the_setting_is_off_by_default_and_nothing_is_ever_requested() {
        let fixture = Fixture::new(None);
        let mut checker = fixture.checker(Ok("v9.0.0"));
        for tick in 0..5 {
            assert!(!checker.tick(at(tick * DAY)));
        }
        assert_eq!(fixture.calls(), 0);
        assert_eq!(
            fixture
                .store
                .section::<UpdateCheckRecord>(UPDATE_CHECK_SECTION),
            None
        );
        let status = fixture.status("2.8.0");
        assert!(!status.enabled);
        assert_eq!(status.update_available, None);
    }

    #[test]
    fn a_newer_release_is_stored_and_offered() {
        let fixture = Fixture::new(None);
        fixture.set_enabled(true);
        let mut checker = fixture.checker(Ok("v2.9.0"));
        assert!(checker.tick(at(0)));
        assert_eq!(fixture.calls(), 1);
        let status = fixture.status("2.8.0 (dev abc123)");
        assert!(status.enabled);
        assert_eq!(status.checked_at.as_deref(), Some("2027-01-15T08:00:00Z"));
        assert_eq!(
            status.update_available,
            Some(LatestRelease {
                version: "2.9.0".into(),
                release_url: "https://github.com/BatterWorks/Hatchdoor/releases/tag/v2.9.0".into(),
            })
        );
    }

    #[test]
    fn the_same_or_an_older_release_offers_nothing() {
        for (tag, running) in [
            ("v2.8.0", "2.8.0"),
            ("v2.7.3", "2.8.0"),
            ("2.8.0", "2.8.0 (dev abc)"),
        ] {
            let fixture = Fixture::new(None);
            fixture.set_enabled(true);
            assert!(fixture.checker(Ok(tag)).tick(at(0)));
            assert_eq!(
                fixture.status(running).update_available,
                None,
                "{tag} on {running}"
            );
        }
    }

    #[test]
    fn a_failure_offers_nothing_and_is_retried_the_next_day() {
        let fixture = Fixture::new(None);
        fixture.set_enabled(true);
        let mut checker = fixture.checker(Err("connection refused"));
        assert!(checker.tick(at(0)));
        assert!(!checker.tick(at(HOUR)));
        assert!(!checker.tick(at(DAY - 1)));
        assert_eq!(fixture.calls(), 1);
        let status = fixture.status("2.8.0");
        assert_eq!(status.update_available, None);
        assert!(status.checked_at.is_some());
        assert!(checker.tick(at(DAY)));
        assert_eq!(fixture.calls(), 2);
    }

    #[test]
    fn a_failure_withdraws_the_release_an_earlier_check_found() {
        let fixture = Fixture::new(None);
        fixture.set_enabled(true);
        assert!(fixture.checker(Ok("v2.9.0")).tick(at(0)));
        assert!(fixture.checker(Err("timed out")).tick(at(DAY)));
        assert_eq!(fixture.status("2.8.0").update_available, None);
    }

    #[test]
    fn an_unreadable_tag_is_not_offered() {
        let fixture = Fixture::new(None);
        fixture.set_enabled(true);
        for tag in [
            "v3.0.0-rc.1",
            "nightly",
            "v3.0",
            "v3.0.0.1",
            "v3.0.0/../../evil",
            "",
        ] {
            fixture.checker(Ok(tag)).tick(at(0));
            assert_eq!(fixture.status("2.8.0").update_available, None, "{tag}");
        }
    }

    #[test]
    fn a_restart_does_not_check_again_within_the_day() {
        let fixture = Fixture::new(None);
        fixture.set_enabled(true);
        assert!(fixture.checker(Ok("v2.9.0")).tick(at(0)));
        let mut after_restart = fixture.checker(Ok("v2.9.0"));
        assert!(!after_restart.tick(at(HOUR)));
        assert!(after_restart.tick(at(DAY)));
        assert_eq!(fixture.calls(), 2);
    }

    #[test]
    fn turning_the_setting_off_stops_the_check_and_hides_the_offer() {
        let fixture = Fixture::new(None);
        fixture.set_enabled(true);
        let mut checker = fixture.checker(Ok("v2.9.0"));
        assert!(checker.tick(at(0)));
        fixture.set_enabled(false);
        assert_eq!(fixture.status("2.8.0").update_available, None);
        assert!(!checker.tick(at(DAY)));
        assert!(!checker.tick(at(5 * DAY)));
        assert_eq!(fixture.calls(), 1);
    }

    #[test]
    fn turning_the_setting_on_checks_at_the_next_tick() {
        let fixture = Fixture::new(None);
        fixture.set_enabled(true);
        let mut checker = fixture.checker(Ok("v2.9.0"));
        assert!(checker.tick(at(0)));
        fixture.set_enabled(false);
        assert!(!checker.tick(at(HOUR)));
        fixture.set_enabled(true);
        assert!(checker.tick(at(HOUR + 60)));
        assert_eq!(fixture.calls(), 2);
        assert!(!checker.tick(at(HOUR + 120)));
    }

    #[test]
    fn a_state_file_that_cannot_be_written_still_checks_once_a_day() {
        let dir = tempfile::tempdir().unwrap();
        let runtime_config = RuntimeConfig::load(
            dir.path().join("settings.json"),
            Environment::from_values([(UPDATE_CHECK_SETTING.to_string(), "true".to_string())]),
            crate::runtime_config::live_settings_defaults(),
        )
        .unwrap();
        // A directory where the file should be: every write fails.
        let blocked = dir.path().join("instance.json");
        std::fs::create_dir(&blocked).unwrap();
        let calls = Arc::new(AtomicUsize::new(0));
        let counted = calls.clone();
        let mut checker = UpdateChecker::new(
            runtime_config,
            InstanceStateStore::new(blocked),
            Arc::new(move || {
                counted.fetch_add(1, Ordering::SeqCst);
                Ok("v2.9.0".into())
            }),
        );
        assert!(checker.tick(at(0)));
        for minute in 1..10 {
            assert!(!checker.tick(at(minute * 60)));
        }
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn a_stored_check_from_the_future_does_not_hold_checks_off() {
        let fixture = Fixture::new(None);
        fixture.set_enabled(true);
        fixture
            .store
            .write_section(
                UPDATE_CHECK_SECTION,
                &UpdateCheckRecord {
                    checked_at: rfc3339(at(30 * DAY)),
                    latest: None,
                },
            )
            .unwrap();
        assert!(fixture.checker(Ok("v2.9.0")).tick(at(0)));
    }

    #[test]
    fn an_environment_pin_is_honoured() {
        let fixture = Fixture::new(Some("true"));
        assert!(fixture.checker(Ok("v2.9.0")).tick(at(0)));
        let fixture = Fixture::new(Some("false"));
        assert!(!fixture.checker(Ok("v2.9.0")).tick(at(0)));
        assert_eq!(fixture.calls(), 0);
    }

    #[test]
    fn versions_compare_by_number() {
        assert!(is_newer("2.10.0", "2.9.9"));
        assert!(is_newer("3.0.0", "2.99.99"));
        assert!(!is_newer("2.9.0", "2.10.0"));
        assert!(!is_newer("2.9.0", "not a version"));
    }
}
