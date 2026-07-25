use std::sync::Mutex;

use fastembed::{EmbeddingModel, InitOptions, TextEmbedding};

use super::Embedder;

pub struct FastembedEmbedder {
    // FastEmbed 5 dropped Rayon; `TextEmbedding::embed` now takes `&mut self`.
    // The `Embedder` contract is `&self` behind a shared `Arc`, so serialize
    // inference through a Mutex rather than leaking `&mut` up the trait.
    model: Mutex<TextEmbedding>,
    dim: usize,
    max_length: usize,
    id: &'static str,
    doc_prefix: &'static str,
    query_prefix: &'static str,
}

impl FastembedEmbedder {
    fn load(
        model: EmbeddingModel,
        dim: usize,
        max_length: usize,
        id: &'static str,
        doc_prefix: &'static str,
        query_prefix: &'static str,
    ) -> Result<Self, String> {
        let model = TextEmbedding::try_new(
            InitOptions::new(model)
                .with_max_length(max_length)
                .with_show_download_progress(false),
        )
        .map_err(|e| format!("failed to load embedding model {id}: {e}"))?;
        Ok(Self {
            model: Mutex::new(model),
            dim,
            max_length,
            id,
            doc_prefix,
            query_prefix,
        })
    }

    pub fn id(&self) -> &'static str {
        self.id
    }

    pub fn bge_small() -> Result<Self, String> {
        Self::load(
            EmbeddingModel::BGESmallENV15,
            384,
            512,
            "BGESmallENV15",
            "",
            "",
        )
    }

    pub fn nomic_v1_5() -> Result<Self, String> {
        // Nomic v1.5 requires task-instruction prefixes; the model is trained
        // to produce different geometries for documents vs. queries.
        Self::load(
            EmbeddingModel::NomicEmbedTextV15,
            768,
            1024,
            "NomicEmbedTextV15",
            "search_document: ",
            "search_query: ",
        )
    }

    pub fn mxbai_large() -> Result<Self, String> {
        Self::load(
            EmbeddingModel::MxbaiEmbedLargeV1,
            1024,
            512,
            "MxbaiEmbedLargeV1",
            "",
            "",
        )
    }
}

impl Embedder for FastembedEmbedder {
    fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, String> {
        if texts.is_empty() {
            return Ok(Vec::new());
        }
        self.model
            .lock()
            .map_err(|e| format!("embedder mutex poisoned: {e}"))?
            .embed(texts, None)
            .map_err(|e| format!("embed call failed: {e}"))
    }

    fn embedding_dim(&self) -> usize {
        self.dim
    }

    fn identity(&self) -> String {
        embedder_identity(self.id, self.dim, self.max_length)
    }

    fn token_count(&self, text: &str, add_special_tokens: bool) -> Result<usize, String> {
        self.model
            .lock()
            .map_err(|e| format!("embedder mutex poisoned: {e}"))?
            .tokenizer
            .encode(text, add_special_tokens)
            .map(|encoding| encoding.get_ids().len())
            .map_err(|error| format!("failed tokenizing text: {error}"))
    }

    fn doc_prefix(&self) -> &'static str {
        self.doc_prefix
    }

    fn query_prefix(&self) -> &'static str {
        self.query_prefix
    }
}

/// Persisted cache identity for a FastEmbed model. Every field that changes the
/// produced vectors is encoded so a mismatch forces a full rebuild rather than
/// mixing incompatible embeddings in one index:
/// - model id and dimension: the embedding space itself;
/// - max sequence length: truncation point;
/// - `fastembed-v5`: the FastEmbed/ONNX Runtime generation;
/// - `ctx1`: the document contract (title + heading header, see
///   `contextual_document` in the cache populate path).
fn embedder_identity(id: &str, dim: usize, max_length: usize) -> String {
    format!("{id}-{dim}-max{max_length}-fastembed-v5-ctx1")
}

#[cfg(test)]
mod identity_tests {
    use super::embedder_identity;

    #[test]
    fn identity_marks_the_contextual_embedding_contract() {
        // The `-ctx1` marker records that documents are embedded with a
        // title/heading header. Bumping it forces a one-time cache rebuild so
        // vectors from the pre-context contract are never reused.
        assert_eq!(
            embedder_identity("NomicEmbedTextV15", 768, 2048),
            "NomicEmbedTextV15-768-max2048-fastembed-v5-ctx1"
        );
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
