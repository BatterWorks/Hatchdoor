//! Candle-backed embedders for FastEmbed v5's Qwen3 and Nomic v2 MoE models,
//! which are not part of the ONNX `EmbeddingModel` enum and are only available
//! behind the `qwen3` / `nomic-v2-moe` feature flags. Each wraps the fastembed
//! candle type behind a `Mutex` (inference is `&mut`-free but the model is not
//! guaranteed `Sync`) plus a separately-loaded tokenizer for `token_count`
//! (the fastembed candle types don't expose their internal tokenizer).
//!
//! Native output dimensions are fixed by the model; the `--dim` sweep applies
//! [`super::MatryoshkaEmbedder`] on top for the reduced-dimension variants.

use std::sync::Mutex;

use candle_core::{DType, Device};
use fastembed::{NomicV2MoeTextEmbedding, Qwen3TextEmbedding};
use tokenizers::Tokenizer;

use super::{Embedder, hub};

/// Cache identity for a candle embedder. Mirrors the ONNX scheme but carries a
/// `candle` marker so a candle-built index is never reused as if it held vectors
/// from the (different) ONNX runtime, and `-ctx1` records the contextual
/// document contract, exactly as `fastembed_embedder`.
fn candle_identity(id: &str, dim: usize, max_length: usize) -> String {
    format!("{id}-{dim}-max{max_length}-fastembed-v5-candle-ctx1")
}

/// Qwen3 Embedding 0.6B — multilingual quality ceiling. Native 1024-dim output
/// with last-token pooling. No task prefixes: Qwen3's optional query instruction
/// is tuning we keep off for a clean benchmark baseline.
pub struct Qwen3Embedder {
    model: Mutex<Qwen3TextEmbedding>,
    tokenizer: Tokenizer,
    dim: usize,
    max_length: usize,
    id: &'static str,
}

impl Qwen3Embedder {
    const REPO: &'static str = "Qwen/Qwen3-Embedding-0.6B";
    const DIM: usize = 1024;
    const MAX_LENGTH: usize = 1024;

    pub fn load() -> Result<Self, String> {
        let model =
            Qwen3TextEmbedding::from_hf(Self::REPO, &Device::Cpu, DType::F32, Self::MAX_LENGTH)
                .map_err(|e| format!("failed to load Qwen3 embedding model: {e}"))?;
        let tokenizer = hub::fetch_tokenizer(Self::REPO)?;
        Ok(Self {
            model: Mutex::new(model),
            tokenizer,
            dim: Self::DIM,
            max_length: Self::MAX_LENGTH,
            id: "Qwen3Embedding0_6B",
        })
    }
}

impl Embedder for Qwen3Embedder {
    fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, String> {
        if texts.is_empty() {
            return Ok(Vec::new());
        }
        self.model
            .lock()
            .map_err(|e| format!("Qwen3 embedder mutex poisoned: {e}"))?
            .embed(texts)
            .map_err(|e| format!("Qwen3 embed call failed: {e}"))
    }

    fn embedding_dim(&self) -> usize {
        self.dim
    }

    fn identity(&self) -> String {
        candle_identity(self.id, self.dim, self.max_length)
    }

    fn token_count(&self, text: &str, add_special_tokens: bool) -> Result<usize, String> {
        self.tokenizer
            .encode(text, add_special_tokens)
            .map(|encoding| encoding.get_ids().len())
            .map_err(|error| format!("failed tokenizing text: {error}"))
    }
}

/// Nomic Embed Text v2 MoE — midsize multilingual, Apache 2.0. Native 768-dim
/// output. Uses the same asymmetric prefixes as v1.5.
///
/// NOTE: the model's effective input limit is 512 tokens, so at the 800-token
/// chunk config the model truncates the tail — a real property of this model to
/// keep in mind when reading its numbers, not a harness bug.
pub struct NomicV2Embedder {
    model: Mutex<NomicV2MoeTextEmbedding>,
    tokenizer: Tokenizer,
    dim: usize,
    max_length: usize,
    id: &'static str,
}

impl NomicV2Embedder {
    const REPO: &'static str = "nomic-ai/nomic-embed-text-v2-moe";
    const DIM: usize = 768;
    const MAX_LENGTH: usize = 512;

    pub fn load() -> Result<Self, String> {
        let model = NomicV2MoeTextEmbedding::from_hf(
            Self::REPO,
            &Device::Cpu,
            DType::F32,
            Self::MAX_LENGTH,
        )
        .map_err(|e| format!("failed to load Nomic v2 MoE embedding model: {e}"))?;
        let tokenizer = hub::fetch_tokenizer(Self::REPO)?;
        Ok(Self {
            model: Mutex::new(model),
            tokenizer,
            dim: Self::DIM,
            max_length: Self::MAX_LENGTH,
            id: "NomicEmbedTextV2Moe",
        })
    }
}

impl Embedder for NomicV2Embedder {
    fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, String> {
        if texts.is_empty() {
            return Ok(Vec::new());
        }
        self.model
            .lock()
            .map_err(|e| format!("Nomic v2 embedder mutex poisoned: {e}"))?
            .embed(texts)
            .map_err(|e| format!("Nomic v2 embed call failed: {e}"))
    }

    fn embedding_dim(&self) -> usize {
        self.dim
    }

    fn identity(&self) -> String {
        candle_identity(self.id, self.dim, self.max_length)
    }

    fn token_count(&self, text: &str, add_special_tokens: bool) -> Result<usize, String> {
        self.tokenizer
            .encode(text, add_special_tokens)
            .map(|encoding| encoding.get_ids().len())
            .map_err(|error| format!("failed tokenizing text: {error}"))
    }

    fn doc_prefix(&self) -> &'static str {
        "search_document: "
    }

    fn query_prefix(&self) -> &'static str {
        "search_query: "
    }
}
