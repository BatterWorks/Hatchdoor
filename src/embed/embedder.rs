use std::sync::Arc;

use tokenizers::{Tokenizer, models::wordlevel::WordLevel, pre_tokenizers::whitespace::Whitespace};

/// In-process text embedder. Loaded once at startup, shared via Arc.
#[allow(dead_code)]
pub trait Embedder: Send + Sync {
    /// Returns one embedding per input string, in order.
    fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, String>;

    /// Embedding dimensionality. Must be constant for the lifetime of the embedder.
    fn embedding_dim(&self) -> usize;

    /// Stable identity of this embedding model (name + dimension). Persisted with
    /// the cache so a model swap can be detected: reusing vectors produced by a
    /// different model would silently mix incompatible embedding spaces in one
    /// vector index. The default is deliberately opaque; real embedders override
    /// it with their model id.
    fn identity(&self) -> String {
        format!("unknown-{}", self.embedding_dim())
    }

    /// The exact tokenizer the embedder uses internally, so the chunker can
    /// pre-compute token counts that match the embedder's accounting.
    fn tokenizer(&self) -> Arc<Tokenizer>;

    /// Task-instruction prefix prepended to documents before embedding.
    /// Empty for models that don't use prefixes; required by Nomic v1.5.
    fn doc_prefix(&self) -> &'static str {
        ""
    }

    /// Task-instruction prefix prepended to queries before embedding.
    fn query_prefix(&self) -> &'static str {
        ""
    }
}

/// Deterministic test embedder. Hashes each input to a fixed-dim vector so
/// tests can assert exact output without loading a real model.
#[allow(dead_code)]
pub struct StubEmbedder {
    dim: usize,
    tokenizer: Arc<Tokenizer>,
}

impl StubEmbedder {
    #[allow(dead_code)]
    pub fn new(dim: usize) -> Self {
        use ahash::AHashMap;
        // WordLevel requires the unk token to be present in the vocab map.
        let mut vocab: AHashMap<String, u32> = AHashMap::new();
        vocab.insert("[UNK]".to_string(), 0);
        let model = WordLevel::builder()
            .vocab(vocab)
            .unk_token("[UNK]".to_string())
            .build()
            .expect("wordlevel model");
        let mut tokenizer = Tokenizer::new(model);
        tokenizer.with_pre_tokenizer(Some(Whitespace {}));
        Self {
            dim,
            tokenizer: Arc::new(tokenizer),
        }
    }
}

impl Embedder for StubEmbedder {
    fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, String> {
        Ok(texts.iter().map(|t| hash_to_vector(t, self.dim)).collect())
    }

    fn embedding_dim(&self) -> usize {
        self.dim
    }

    fn identity(&self) -> String {
        format!("stub-{}", self.dim)
    }

    fn tokenizer(&self) -> Arc<Tokenizer> {
        self.tokenizer.clone()
    }
}

#[allow(dead_code)]
fn hash_to_vector(input: &str, dim: usize) -> Vec<f32> {
    let mut hasher = blake3::Hasher::new();
    hasher.update(input.as_bytes());
    let mut output = hasher.finalize_xof();

    let mut vector = Vec::with_capacity(dim);
    let mut bytes = [0u8; 4];
    for _ in 0..dim {
        output.fill(&mut bytes);
        let v = (u32::from_le_bytes(bytes) as f64 / u32::MAX as f64) * 2.0 - 1.0;
        vector.push(v as f32);
    }
    let norm = vector.iter().map(|v| v * v).sum::<f32>().sqrt().max(1e-12);
    for v in &mut vector {
        *v /= norm;
    }
    vector
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stub_embedder_produces_fixed_dim_vectors() {
        let embedder = StubEmbedder::new(384);
        let vectors = embedder
            .embed(&["hello".to_string(), "world".to_string()])
            .expect("embed");
        assert_eq!(vectors.len(), 2);
        assert_eq!(vectors[0].len(), 384);
        assert_eq!(vectors[1].len(), 384);
    }

    #[test]
    fn stub_embedder_is_deterministic_for_identical_input() {
        let embedder = StubEmbedder::new(384);
        let a = embedder.embed(&["hello".to_string()]).expect("embed");
        let b = embedder.embed(&["hello".to_string()]).expect("embed");
        assert_eq!(a, b);
    }

    #[test]
    fn stub_embedder_distinguishes_different_inputs() {
        let embedder = StubEmbedder::new(384);
        let a = embedder.embed(&["hello".to_string()]).expect("embed");
        let b = embedder.embed(&["world".to_string()]).expect("embed");
        assert_ne!(a, b);
    }

    #[test]
    fn stub_embedder_reports_its_dim() {
        let embedder = StubEmbedder::new(384);
        assert_eq!(embedder.embedding_dim(), 384);
    }

    #[test]
    fn stub_tokenizer_counts_whitespace_tokens() {
        let embedder = StubEmbedder::new(384);
        let tokenizer = embedder.tokenizer();
        let encoding = tokenizer.encode("hello world foo", false).expect("encode");
        assert_eq!(encoding.get_ids().len(), 3);
    }
}
