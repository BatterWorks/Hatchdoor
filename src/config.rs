//! Environment-driven application configuration and logging setup. Parsed once
//! at startup; the resulting values are threaded into `AppState`.

use std::env;
use std::net::SocketAddr;
use std::path::PathBuf;

use tracing_subscriber::EnvFilter;

#[derive(Debug, Clone)]
pub struct AppConfig {
    pub vault_path: PathBuf,
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
        let vault_path = env::var("VAULT_PATH").unwrap_or_else(|_| "./vault".to_string());
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
            vault_path: PathBuf::from(vault_path),
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
        format!("{}:{}", self.host, self.port)
            .parse::<SocketAddr>()
            .map_err(|e| format!("invalid bind address: {e}"))
    }
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
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("hatchdoor=info,tower_http=info,axum::rejection=warn"));

    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(false)
        .compact()
        .init();
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn socket_addr_builds_expected_address() {
        let cfg = AppConfig {
            vault_path: PathBuf::from("./vault"),
            cache_db_path: PathBuf::from("./data/cache/hatchdoor-cache.sqlite3"),
            host: "0.0.0.0".to_string(),
            port: 42824,
            web_bearer_token: None,
            demo_mode: true,
            archive_prefix: "90-archive/".to_string(),
            exclude_patterns: Vec::new(),
            embed_layers: true,
        };

        let addr = cfg.socket_addr().expect("valid addr");
        assert_eq!(addr.to_string(), "0.0.0.0:42824");
    }
}
