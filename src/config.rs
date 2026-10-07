//! Environment-driven application configuration and logging setup. Parsed once
//! at startup; the resulting values are threaded into `AppState`.

use std::env;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::path::PathBuf;

use tracing_subscriber::EnvFilter;

use crate::vault_runtime::VaultSource;

/// Whether this build is a published release. A build is one only when it
/// was told so: the image build passed its `VERSION` build argument, compiled
/// in as `HATCHDOOR_RELEASE_VERSION`, and it names this crate's own version.
/// A nightly image and a build from source are not, whatever else they pass.
fn is_release_build() -> bool {
    is_release(
        env!("CARGO_PKG_VERSION"),
        option_env!("HATCHDOOR_RELEASE_VERSION"),
    )
}

fn is_release(crate_version: &str, build_argument: Option<&str>) -> bool {
    build_argument.map(str::trim) == Some(crate_version)
}

/// The version reported to clients and logs. A release reports the plain
/// crate version, even when its image build passed the commit for the
/// image's `revision` label. Any other build that was passed a commit,
/// through the `HATCHDOOR_GIT_SHA` compile-time env var (fed by the `GIT_SHA`
/// build argument), reports it as `2.8.0 (dev abc123)`.
pub fn version_string() -> String {
    describe_version(
        env!("CARGO_PKG_VERSION"),
        is_release_build(),
        option_env!("HATCHDOOR_GIT_SHA"),
    )
}

fn describe_version(crate_version: &str, release: bool, commit: Option<&str>) -> String {
    match commit.map(str::trim) {
        Some(commit) if !release && !commit.is_empty() => {
            format!("{crate_version} (dev {commit})")
        }
        _ => crate_version.to_string(),
    }
}

/// The build's channel, for the usage report (ADR-45): `stable` for a
/// release and `dev` for every other build.
pub fn build_channel() -> &'static str {
    channel_word(is_release_build())
}

fn channel_word(release: bool) -> &'static str {
    if release { "stable" } else { "dev" }
}

/// How this build was packaged, for the usage report (ADR-45): `docker` or
/// `podman` when the image build passed that word as the `HATCHDOOR_IMAGE`
/// build argument, and `source` for every other build.
pub fn build_image() -> &'static str {
    image_word(option_env!("HATCHDOOR_IMAGE"))
}

fn image_word(build_argument: Option<&str>) -> &'static str {
    match build_argument.map(str::trim) {
        Some("docker") => "docker",
        Some("podman") => "podman",
        _ => "source",
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LegacyVaultEnvironmentKeyKind {
    DeploymentPath,
    Migrated,
    Retired,
    RemovedStartupSource,
}

pub(crate) fn legacy_vault_environment_key_kind(
    key: &str,
) -> Option<LegacyVaultEnvironmentKeyKind> {
    match key {
        "VAULT_PATH" => Some(LegacyVaultEnvironmentKeyKind::DeploymentPath),
        "HATCHDOOR_EXCLUDE" => Some(LegacyVaultEnvironmentKeyKind::Migrated),
        "HATCHDOOR_GIT_DEBOUNCE_SECONDS" => Some(LegacyVaultEnvironmentKeyKind::Retired),
        "HATCHDOOR_VAULT_SOURCE" => Some(LegacyVaultEnvironmentKeyKind::RemovedStartupSource),
        _ if key.starts_with("HATCHDOOR_VAULT_GIT_") => {
            Some(LegacyVaultEnvironmentKeyKind::RemovedStartupSource)
        }
        _ if key.starts_with("HATCHDOOR_GIT_") => Some(LegacyVaultEnvironmentKeyKind::Migrated),
        _ => None,
    }
}

fn removed_startup_source_environment_keys(
    environment: impl IntoIterator<Item = (String, String)>,
) -> Vec<String> {
    environment
        .into_iter()
        .filter(|(key, value)| {
            !value.trim().is_empty()
                && legacy_vault_environment_key_kind(key)
                    == Some(LegacyVaultEnvironmentKeyKind::RemovedStartupSource)
        })
        .map(|(key, _)| key)
        .collect()
}

#[derive(Debug, Clone)]
pub struct AppConfig {
    pub vault_source: VaultSource,
    pub cache_db_path: PathBuf,
    pub host: String,
    pub port: u16,
    /// When set, every `/api/*`, asset, and download request must present this
    /// token (Bearer header or `access_token` query parameter).
    pub web_bearer_token: Option<String>,
    /// Public demo mode: allows unauthenticated public browsing while disabling
    /// every app-level write surface.
    pub demo_mode: bool,
    /// Folder prefix (with trailing slash) treated as archived in resolve results.
    pub archive_prefix: String,
    /// Extra noise-exclusion patterns from `HATCHDOOR_EXCLUDE` (gitignore
    /// syntax), appended to the built-in defaults. Empty when unset.
    pub exclude_patterns: Vec<String>,
    /// Whether demoted-layer vectors are embedded (`HATCHDOOR_EMBED_LAYERS`,
    /// default true). Surfaced here only for the effective-config startup log;
    /// the cache reads the same env var when it persists the value.
    pub embed_layers: bool,
}

impl AppConfig {
    pub fn from_env() -> Result<Self, String> {
        let removed_source_keys = removed_startup_source_environment_keys(env::vars());
        if !removed_source_keys.is_empty() {
            return Err(format!(
                "These development-only managed-startup variables were removed: {}. Remove them and configure the Git-backed Vault in Settings or with the edit_vault MCP tool.",
                removed_source_keys.join(", ")
            ));
        }
        let vault_path = env::var("VAULT_PATH").unwrap_or_else(|_| "./vault".to_string());
        let vault_source = VaultSource::Local {
            vault_path: PathBuf::from(vault_path),
        };
        let cache_db_path = env::var("HATCHDOOR_CACHE_DB")
            .unwrap_or_else(|_| "./data/cache/hatchdoor-cache.sqlite3".to_string());
        let host = env::var("HOST").unwrap_or_else(|_| "127.0.0.1".to_string());
        let port_raw = env::var("PORT").unwrap_or_else(|_| "42824".to_string());
        let web_bearer_token = env::var("HATCHDOOR_WEB_BEARER_TOKEN")
            .ok()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty());
        let demo_mode = env::var("HATCHDOOR_DEMO_MODE")
            .map(|value| crate::runtime_config::is_truthy(&value))
            .unwrap_or(false);
        let port = parse_port(&port_raw)?;

        Ok(Self {
            vault_source,
            cache_db_path: PathBuf::from(cache_db_path),
            host,
            port,
            web_bearer_token,
            demo_mode,
            archive_prefix: "90-archive/".to_string(),
            exclude_patterns: Vec::new(),
            embed_layers: true,
        })
    }

    /// Apply the already-resolved live settings. Environment capture and store
    /// precedence live in `RuntimeConfig`; this layer retains only typed
    /// interpretation of the values it consumes.
    pub fn apply_runtime_snapshot(
        &mut self,
        snapshot: &crate::runtime_config::ConfigSnapshot,
    ) -> Result<(), String> {
        self.archive_prefix = snapshot
            .required("HATCHDOOR_ARCHIVE_PREFIX")?
            .trim()
            .to_string();
        self.exclude_patterns = parse_exclude_patterns(snapshot.required("HATCHDOOR_EXCLUDE")?);
        self.embed_layers =
            crate::runtime_config::is_truthy(snapshot.required("HATCHDOOR_EMBED_LAYERS")?);
        Ok(())
    }

    pub fn socket_addr(&self) -> Result<SocketAddr, String> {
        socket_addr_for_host(&self.host, self.port)
    }
}

/// Convert the supported `HOST` forms to a socket address without DNS lookup.
/// `localhost` is deliberately the IPv4 loopback alias; other hostnames are
/// rejected so startup and the container health probe have one address family.
pub fn socket_addr_for_host(host: &str, port: u16) -> Result<SocketAddr, String> {
    let host = host.trim();
    let normalized = match host {
        "localhost" => IpAddr::V4(Ipv4Addr::LOCALHOST),
        _ => host
            .strip_prefix('[')
            .and_then(|value| value.strip_suffix(']'))
            .unwrap_or(host)
            .parse::<IpAddr>()
            .map_err(|_| {
                format!(
                    "invalid HOST '{host}': use an IP literal or localhost; hostname resolution is not supported"
                )
            })?,
    };
    Ok(SocketAddr::new(normalized, port))
}

/// Choose the local probe address that matches the selected listener family.
pub fn healthcheck_socket_addr(host: &str, port: u16) -> Result<SocketAddr, String> {
    let listener = socket_addr_for_host(host, port)?;
    let loopback = match listener.ip() {
        IpAddr::V4(_) => IpAddr::V4(Ipv4Addr::LOCALHOST),
        IpAddr::V6(_) => IpAddr::V6(Ipv6Addr::LOCALHOST),
    };
    Ok(SocketAddr::new(loopback, port))
}

pub fn parse_port(input: &str) -> Result<u16, String> {
    input
        .parse::<u16>()
        .map_err(|e| format!("invalid PORT '{input}': {e}"))
}

/// Split a comma-separated `HATCHDOOR_EXCLUDE` value into individual gitignore
/// patterns, trimming surrounding whitespace and dropping empty entries so a
/// trailing comma or accidental double comma does not produce a blank pattern.
pub fn parse_exclude_patterns(raw: &str) -> Vec<String> {
    raw.split(',')
        .map(|pattern| pattern.trim())
        .filter(|pattern| !pattern.is_empty())
        .map(|pattern| pattern.to_string())
        .collect()
}

pub fn init_logging() {
    let filter =
        capped_log_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| {
            EnvFilter::new("hatchdoor=info,tower_http=info,axum::rejection=warn")
        }));

    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(false)
        .compact()
        .init();
}

/// The MCP library logs every message it sends, tool results included, at
/// debug and trace. A tool result can carry a transfer link (ADR-27), whose
/// credential must never reach a log, so the library is held at info whatever
/// `RUST_LOG` asks for. A directive for a more specific target wins in an
/// `EnvFilter`, so the two modules that log messages are capped by name as
/// well as the crate. Its info-level lines carry no messages.
const MESSAGE_LOGGING_TARGETS: &[&str] = &[
    "rmcp",
    "rmcp::service",
    "rmcp::transport::streamable_http_server",
];

fn capped_log_filter(filter: EnvFilter) -> EnvFilter {
    MESSAGE_LOGGING_TARGETS
        .iter()
        .fold(filter, |filter, target| {
            filter.add_directive(
                format!("{target}=info")
                    .parse()
                    .expect("a static log directive parses"),
            )
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_build_reports_docker_or_podman_only_when_told_and_source_otherwise() {
        assert_eq!(image_word(Some("docker")), "docker");
        assert_eq!(image_word(Some(" podman ")), "podman");
        assert_eq!(image_word(Some("")), "source");
        assert_eq!(
            image_word(Some("my-own-registry.example/hatchdoor")),
            "source"
        );
        assert_eq!(image_word(None), "source");
    }

    #[test]
    fn a_build_is_a_release_only_when_told_its_own_version() {
        assert!(is_release("2.8.0", Some("2.8.0")));
        assert!(is_release("2.8.0", Some(" 2.8.0 ")));
        assert!(!is_release("2.8.0", Some("v2.8.0")));
        assert!(!is_release("2.8.0", None));
        assert!(!is_release("2.8.0", Some("")));
        assert!(!is_release("2.8.0", Some("2.8.1")));
        assert!(!is_release("2.8.0", Some("2.8")));
        assert!(!is_release("2.8.0", Some("latest")));
    }

    #[test]
    fn a_release_shows_a_plain_version_and_only_another_build_shows_its_commit() {
        // A release, built with or without the commit the image label needs.
        assert_eq!(describe_version("2.8.0", true, Some("abc123")), "2.8.0");
        assert_eq!(describe_version("2.8.0", true, None), "2.8.0");
        // A nightly image.
        assert_eq!(
            describe_version("2.8.0", false, Some("abc123")),
            "2.8.0 (dev abc123)"
        );
        // Built from source with nothing passed.
        assert_eq!(describe_version("2.8.0", false, None), "2.8.0");
        assert_eq!(describe_version("2.8.0", false, Some("")), "2.8.0");
    }

    #[test]
    fn the_channel_is_stable_for_a_release_and_dev_for_every_other_build() {
        assert_eq!(channel_word(true), "stable");
        assert_eq!(channel_word(false), "dev");
        // This test binary was built with no build arguments.
        assert_eq!(build_channel(), "dev");
    }

    #[test]
    fn message_logging_stays_at_info_even_under_a_trace_filter() {
        let filter = capped_log_filter(EnvFilter::new(
            "trace,rmcp=trace,rmcp::service=trace,rmcp::transport::streamable_http_server=debug",
        ))
        .to_string();
        for target in MESSAGE_LOGGING_TARGETS {
            assert!(filter.contains(&format!("{target}=info")), "{filter}");
            for noisy in ["trace", "debug"] {
                assert!(!filter.contains(&format!("{target}={noisy}")), "{filter}");
            }
        }
    }

    #[test]
    fn parse_port_accepts_valid_u16() {
        assert_eq!(parse_port("42824").expect("valid port"), 42824);
    }

    #[test]
    fn parse_port_rejects_invalid_values() {
        assert!(parse_port("70000").is_err());
        assert!(parse_port("abc").is_err());
    }

    #[test]
    fn parse_exclude_patterns_trims_and_drops_blanks() {
        assert_eq!(
            parse_exclude_patterns("build/, *.log ,, !.DS_Store"),
            vec![
                "build/".to_string(),
                "*.log".to_string(),
                "!.DS_Store".to_string(),
            ]
        );
        assert!(parse_exclude_patterns("   ").is_empty());
        assert!(parse_exclude_patterns("").is_empty());
    }

    #[test]
    fn socket_addr_normalizes_every_accepted_loopback_spelling() {
        for (host, expected) in [
            ("127.0.0.1", "127.0.0.1:42824"),
            ("localhost", "127.0.0.1:42824"),
            ("::1", "[::1]:42824"),
            ("[::1]", "[::1]:42824"),
        ] {
            let cfg = AppConfig {
                vault_source: VaultSource::Local {
                    vault_path: PathBuf::from("./vault"),
                },
                cache_db_path: PathBuf::from("./data/cache/hatchdoor-cache.sqlite3"),
                host: host.to_string(),
                port: 42824,
                web_bearer_token: None,
                demo_mode: true,
                archive_prefix: "90-archive/".to_string(),
                exclude_patterns: Vec::new(),
                embed_layers: true,
            };

            assert_eq!(cfg.socket_addr().expect(host).to_string(), expected);
        }
    }

    #[test]
    fn socket_addr_rejects_unsupported_hostnames_with_guidance() {
        let error = socket_addr_for_host("hatchdoor.example", 42824)
            .expect_err("hostname resolution is deliberately unsupported");
        assert!(error.contains("HOST 'hatchdoor.example'"));
        assert!(error.contains("IP literal or localhost"));
    }

    #[test]
    fn healthcheck_target_matches_listener_address_family() {
        assert_eq!(
            healthcheck_socket_addr("localhost", 42824)
                .expect("IPv4 loopback target")
                .to_string(),
            "127.0.0.1:42824"
        );
        assert_eq!(
            healthcheck_socket_addr("[::1]", 42824)
                .expect("IPv6 loopback target")
                .to_string(),
            "[::1]:42824"
        );
    }

    #[test]
    fn removed_managed_startup_variables_are_named_together() {
        let keys = removed_startup_source_environment_keys([
            ("HATCHDOOR_VAULT_SOURCE".to_string(), "git".to_string()),
            (
                "HATCHDOOR_VAULT_GIT_URL".to_string(),
                "https://example.test/vault.git".to_string(),
            ),
            ("HATCHDOOR_VAULT_GIT_MODE".to_string(), " ".to_string()),
            ("HATCHDOOR_GIT_BRANCH".to_string(), "main".to_string()),
        ]);

        assert_eq!(
            keys,
            vec!["HATCHDOOR_VAULT_SOURCE", "HATCHDOOR_VAULT_GIT_URL"]
        );
    }
}
