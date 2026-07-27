//! Hugging Face Hub helpers for the benchmark-only embedders that load their own
//! assets: the candle models (Qwen3, Nomic v2 MoE) need a tokenizer for
//! `token_count` because the fastembed candle types don't expose their internal
//! one, and the Arctic user-defined ONNX model needs its raw ONNX + tokenizer
//! files. Uses the same `hf-hub` sync API and cache directory FastEmbed uses, so
//! files fetched here are shared with — not duplicated by — the model loads.

use hf_hub::api::sync::{ApiBuilder, ApiRepo};
use tokenizers::Tokenizer;

fn repo(repo_id: &str) -> Result<ApiRepo, String> {
    let api = ApiBuilder::new()
        .with_progress(false)
        .build()
        .map_err(|e| format!("hf-hub api init: {e}"))?;
    Ok(api.model(repo_id.to_string()))
}

/// Fetch a single file from a repo into the shared Hugging Face cache and
/// return its path. Large ONNX files should be passed to ONNX Runtime by path
/// rather than copied into the process heap first.
pub fn fetch_path(repo_id: &str, filename: &str) -> Result<std::path::PathBuf, String> {
    repo(repo_id)?
        .get(filename)
        .map_err(|e| format!("hf-hub get {repo_id}/{filename}: {e}"))
}

/// Fetch a single file from a repo, returning its bytes (cached on disk by hf-hub).
pub fn fetch_bytes(repo_id: &str, filename: &str) -> Result<Vec<u8>, String> {
    let path = fetch_path(repo_id, filename)?;
    std::fs::read(&path).map_err(|e| format!("read {}: {e}", path.display()))
}

/// Load a `tokenizers::Tokenizer` from a repo's tokenizer.json, for token
/// counting during chunking. No truncation is configured so counts are exact.
pub fn fetch_tokenizer(repo_id: &str) -> Result<Tokenizer, String> {
    let path = repo(repo_id)?
        .get("tokenizer.json")
        .map_err(|e| format!("hf-hub get {repo_id}/tokenizer.json: {e}"))?;
    Tokenizer::from_file(&path).map_err(|e| format!("load tokenizer {repo_id}: {e}"))
}
