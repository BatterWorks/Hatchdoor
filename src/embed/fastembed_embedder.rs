use std::sync::Arc;

use fastembed::{EmbeddingModel, InitOptions, TextEmbedding};
use tokenizers::Tokenizer;

use super::Embedder;

pub struct FastembedEmbedder {
    model: TextEmbedding,
    tokenizer: Arc<Tokenizer>,
    dim: usize,
    id: &'static str,
}

impl FastembedEmbedder {
    fn load(model: EmbeddingModel, dim: usize, id: &'static str) -> Result<Self, String> {
        let model = TextEmbedding::try_new(
            InitOptions::new(model).with_show_download_progress(false),
        )
        .map_err(|e| format!("failed to load embedding model {id}: {e}"))?;
        let tokenizer = Arc::new(model.tokenizer.clone());
        Ok(Self { model, tokenizer, dim, id })
    }

    pub fn id(&self) -> &'static str {
        self.id
    }

    pub fn bge_small() -> Result<Self, String> {
        Self::load(EmbeddingModel::BGESmallENV15, 384, "BGESmallENV15")
    }

    pub fn nomic_v1_5() -> Result<Self, String> {
        Self::load(EmbeddingModel::NomicEmbedTextV15, 768, "NomicEmbedTextV15")
    }

    pub fn mxbai_large() -> Result<Self, String> {
        Self::load(EmbeddingModel::MxbaiEmbedLargeV1, 1024, "MxbaiEmbedLargeV1")
    }
}

impl Embedder for FastembedEmbedder {
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
    fn bge_small_has_384_dim_and_correct_id() {
        let e = FastembedEmbedder::bge_small().expect("load");
        assert_eq!(e.embedding_dim(), 384);
        assert_eq!(e.id(), "BGESmallENV15");
    }

    #[test]
    fn nomic_v1_5_has_768_dim_and_correct_id() {
        let e = FastembedEmbedder::nomic_v1_5().expect("load");
        assert_eq!(e.embedding_dim(), 768);
        assert_eq!(e.id(), "NomicEmbedTextV15");
    }

    #[test]
    fn mxbai_large_has_1024_dim_and_correct_id() {
        let e = FastembedEmbedder::mxbai_large().expect("load");
        assert_eq!(e.embedding_dim(), 1024);
        assert_eq!(e.id(), "MxbaiEmbedLargeV1");
    }
}
