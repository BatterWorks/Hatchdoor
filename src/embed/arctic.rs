use std::sync::Arc;

use fastembed::{EmbeddingModel, InitOptions, TextEmbedding};
use tokenizers::Tokenizer;

use super::Embedder;

pub(crate) struct ArcticEmbedder {
    model: TextEmbedding,
    tokenizer: Arc<Tokenizer>,
    dim: usize,
}

// NOTE: fastembed 4.9.1 does not yet ship SnowflakeArcticEmbedS in its
// EmbeddingModel enum.  BGESmallENV15 is the closest available 384-dim
// retrieval-tuned model and is a drop-in stand-in until a fastembed release
// adds Arctic-S.  Swapping back is a one-line change here plus a
// ARCTIC_S_DIM rename.
#[allow(dead_code)]
const ARCTIC_S_DIM: usize = 384;
#[allow(dead_code)]
const EMBEDDING_MODEL: EmbeddingModel = EmbeddingModel::BGESmallENV15;

impl ArcticEmbedder {
    #[allow(dead_code)]
    pub(crate) fn load() -> Result<Self, String> {
        let model = TextEmbedding::try_new(
            InitOptions::new(EMBEDDING_MODEL).with_show_download_progress(false),
        )
        .map_err(|e| format!("failed to load embedding model: {e}"))?;

        // TextEmbedding exposes its internal tokenizer as a public field; we
        // clone it so the chunker can share the exact same tokenizer without
        // loading a second copy from disk.
        let tokenizer = Arc::new(model.tokenizer.clone());

        Ok(Self { model, tokenizer, dim: ARCTIC_S_DIM })
    }
}

impl Embedder for ArcticEmbedder {
    fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, String> {
        if texts.is_empty() {
            return Ok(Vec::new());
        }
        self.model
            .embed(texts.to_vec(), None)
            .map_err(|e| format!("embed call failed: {e}"))
    }

    fn embedding_dim(&self) -> usize {
        self.dim
    }

    fn tokenizer(&self) -> Arc<Tokenizer> {
        self.tokenizer.clone()
    }
}

#[cfg(all(test, feature = "embedder-tests"))]
mod tests {
    use super::*;

    #[test]
    fn arctic_embedder_produces_384_dim_finite_vectors() {
        let embedder = ArcticEmbedder::load().expect("load embedder");
        let vectors = embedder
            .embed(&["hello world".to_string(), "second input".to_string()])
            .expect("embed");
        assert_eq!(vectors.len(), 2);
        assert_eq!(vectors[0].len(), 384);
        assert_eq!(vectors[1].len(), 384);
        assert!(vectors[0].iter().all(|v| v.is_finite()));
        assert_eq!(embedder.embedding_dim(), 384);
    }

    #[test]
    fn arctic_embedder_is_deterministic_for_identical_input() {
        let embedder = ArcticEmbedder::load().expect("load embedder");
        let a = embedder.embed(&["hello".to_string()]).expect("first");
        let b = embedder.embed(&["hello".to_string()]).expect("second");
        assert_eq!(a, b);
    }

    #[test]
    fn arctic_tokenizer_is_loaded_alongside_model() {
        let embedder = ArcticEmbedder::load().expect("load embedder");
        let encoding = embedder
            .tokenizer()
            .encode("hello world", false)
            .expect("encode");
        assert!(!encoding.get_ids().is_empty());
    }
}
