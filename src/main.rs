use dotenvy::dotenv;
use tracing::{error, info};

use hatchdoor::config::init_logging;
use hatchdoor::embed::FastembedEmbedder;
use hatchdoor::server::run_server;

enum RunMode {
    Serve,
    PrefetchEmbedder,
    Healthcheck,
    Unknown(String),
}

fn parse_run_mode(args: &[String]) -> RunMode {
    match args.get(1).map(String::as_str) {
        None => RunMode::Serve,
        Some("--prefetch-embedder") => RunMode::PrefetchEmbedder,
        Some("--healthcheck") => RunMode::Healthcheck,
        Some(other) => RunMode::Unknown(other.to_string()),
    }
}

fn run_prefetch() {
    info!("Pre-fetching Nomic Embed Text v1.5 weights and tokenizer");
    match FastembedEmbedder::nomic_v1_5() {
        Ok(_) => info!("Pre-fetch complete"),
        Err(e) => {
            error!("Pre-fetch failed: {e}");
            std::process::exit(1);
        }
    }
}

#[tokio::main]
async fn main() {
    dotenv().ok();
    init_logging();

    let args: Vec<String> = std::env::args().collect();
    match parse_run_mode(&args) {
        RunMode::Serve => run_server().await,
        RunMode::PrefetchEmbedder => run_prefetch(),
        RunMode::Healthcheck => run_healthcheck(),
        RunMode::Unknown(flag) => {
            error!("Unknown flag: {flag}");
            std::process::exit(2);
        }
    }
}

/// Container health probe: hit the local `/health` endpoint over a raw socket
/// (the distroless runtime has no shell or curl) and exit non-zero on failure.
fn run_healthcheck() {
    use std::io::{Read, Write};

    let port = std::env::var("PORT")
        .ok()
        .and_then(|value| value.parse::<u16>().ok())
        .unwrap_or(42824);
    let addr = format!("127.0.0.1:{port}");

    let probe = || -> std::io::Result<bool> {
        let mut stream = std::net::TcpStream::connect(&addr)?;
        stream.set_read_timeout(Some(std::time::Duration::from_secs(4)))?;
        stream.set_write_timeout(Some(std::time::Duration::from_secs(4)))?;
        stream
            .write_all(b"GET /health HTTP/1.0\r\nHost: localhost\r\nConnection: close\r\n\r\n")?;
        let mut response = String::new();
        stream.read_to_string(&mut response)?;
        let status_ok = response
            .lines()
            .next()
            .map(|line| line.contains(" 200"))
            .unwrap_or(false);
        Ok(status_ok)
    };

    match probe() {
        Ok(true) => std::process::exit(0),
        Ok(false) => {
            eprintln!("healthcheck: endpoint did not report healthy");
            std::process::exit(1);
        }
        Err(error) => {
            eprintln!("healthcheck: {error}");
            std::process::exit(1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cli_recognises_prefetch_embedder_flag() {
        let args = vec!["hatchdoor".to_string(), "--prefetch-embedder".to_string()];
        assert!(matches!(parse_run_mode(&args), RunMode::PrefetchEmbedder));
    }

    #[test]
    fn cli_defaults_to_serve_mode() {
        let args = vec!["hatchdoor".to_string()];
        assert!(matches!(parse_run_mode(&args), RunMode::Serve));
    }

    #[test]
    fn cli_rejects_unknown_flags() {
        let args = vec!["hatchdoor".to_string(), "--bogus".to_string()];
        assert!(matches!(parse_run_mode(&args), RunMode::Unknown(_)));
    }
}
