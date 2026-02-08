mod render;
mod vault;
mod wikilink;

use std::env;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

use axum::Router;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{Html, IntoResponse, Redirect};
use axum::routing::get;
use dotenvy::dotenv;
use tower_http::services::ServeDir;

use crate::render::{markdown_to_html, render_note_page};
use crate::vault::VaultIndex;
use crate::wikilink::rewrite_wikilinks;

#[derive(Debug, Clone)]
struct AppConfig {
    vault_path: PathBuf,
    home_note: String,
    host: String,
    port: u16,
}

impl AppConfig {
    fn from_env() -> Result<Self, String> {
        let vault_path = env::var("VAULT_PATH").unwrap_or_else(|_| "./vault".to_string());
        let home_note = env::var("HOME_NOTE").unwrap_or_else(|_| "Home".to_string());
        let host = env::var("HOST").unwrap_or_else(|_| "0.0.0.0".to_string());
        let port_raw = env::var("PORT").unwrap_or_else(|_| "42824".to_string());
        let port = parse_port(&port_raw)?;

        Ok(Self {
            vault_path: PathBuf::from(vault_path),
            home_note,
            host,
            port,
        })
    }

    fn socket_addr(&self) -> Result<SocketAddr, String> {
        format!("{}:{}", self.host, self.port)
            .parse::<SocketAddr>()
            .map_err(|e| format!("invalid bind address: {e}"))
    }
}

fn parse_port(input: &str) -> Result<u16, String> {
    input
        .parse::<u16>()
        .map_err(|e| format!("invalid PORT '{input}': {e}"))
}

#[derive(Clone)]
struct AppState {
    index: Arc<VaultIndex>,
    home_slug: String,
}

#[tokio::main]
async fn main() {
    dotenv().ok();

    let config = AppConfig::from_env().unwrap_or_else(|e| {
        eprintln!("Configuration error: {e}");
        std::process::exit(1);
    });

    let index = VaultIndex::build(&config.vault_path).unwrap_or_else(|e| {
        eprintln!(
            "Failed to index vault at {}: {e}",
            config.vault_path.display()
        );
        std::process::exit(1);
    });

    let home_slug = index
        .resolve_wikilink(&config.home_note)
        .map(|n| n.slug.clone())
        .unwrap_or_else(|| {
            eprintln!(
                "Home note '{}' not found in vault {}",
                config.home_note,
                index.root().display()
            );
            std::process::exit(1);
        });

    let state = AppState {
        index: Arc::new(index),
        home_slug,
    };

    let app = Router::new()
        .route("/", get(root_handler))
        .route("/health", get(health_handler))
        .route("/n/{slug}", get(note_handler))
        .nest_service("/assets", ServeDir::new("static"))
        .with_state(state);

    let addr = config.socket_addr().unwrap_or_else(|e| {
        eprintln!("Address error: {e}");
        std::process::exit(1);
    });

    println!("Hatchdoor listening on http://{addr}");
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .unwrap_or_else(|e| {
            eprintln!("Failed to bind: {e}");
            std::process::exit(1);
        });

    axum::serve(listener, app).await.unwrap_or_else(|e| {
        eprintln!("Server error: {e}");
        std::process::exit(1);
    });
}

async fn root_handler(State(state): State<AppState>) -> Redirect {
    Redirect::to(&format!("/n/{}", state.home_slug))
}

async fn health_handler() -> impl IntoResponse {
    (StatusCode::OK, "ok")
}

async fn note_handler(
    Path(slug): Path<String>,
    State(state): State<AppState>,
) -> impl IntoResponse {
    match state.index.read_note_by_slug(&slug) {
        Ok(Some(note)) => {
            let rewritten = rewrite_wikilinks(&note.content, &state.index);
            let body = markdown_to_html(&rewritten);
            let page = render_note_page(&note.title, &body);
            (StatusCode::OK, Html(page)).into_response()
        }
        Ok(None) => {
            let body =
                format!("<h1>Not Found</h1><p>No note exists for slug: <code>{slug}</code></p>");
            let page = render_note_page("Not Found", &body);
            (StatusCode::NOT_FOUND, Html(page)).into_response()
        }
        Err(e) => {
            let body = format!(
                "<h1>Error</h1><p>Failed reading note <code>{slug}</code>: {}</p>",
                wikilink::escape_html(&e.to_string())
            );
            let page = render_note_page("Error", &body);
            (StatusCode::INTERNAL_SERVER_ERROR, Html(page)).into_response()
        }
    }
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
    fn socket_addr_builds_expected_address() {
        let cfg = AppConfig {
            vault_path: PathBuf::from("./vault"),
            home_note: "Home".to_string(),
            host: "0.0.0.0".to_string(),
            port: 42824,
        };

        let addr = cfg.socket_addr().expect("valid addr");
        assert_eq!(addr.to_string(), "0.0.0.0:42824");
    }
}
