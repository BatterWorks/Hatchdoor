use std::path::PathBuf;
use std::sync::Mutex;

use fastembed::{
    EmbeddingModel, InitOptions, InitOptionsUserDefined, Pooling, TextEmbedding, TokenizerFiles,
    UserDefinedEmbeddingModel,
};

use super::Embedder;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DocumentFormat {
    HatchdoorContextual,
    GemmaRetrievalV1,
}

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
    document_format: DocumentFormat,
    identity_suffix: &'static str,
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
        Self::load_in(
            model,
            dim,
            max_length,
            id,
            doc_prefix,
            query_prefix,
            fastembed::get_cache_dir().into(),
        )
    }

    fn load_in(
        model: EmbeddingModel,
        dim: usize,
        max_length: usize,
        id: &'static str,
        doc_prefix: &'static str,
        query_prefix: &'static str,
        cache_dir: PathBuf,
    ) -> Result<Self, String> {
        let model = TextEmbedding::try_new(
            InitOptions::new(model)
                .with_max_length(max_length)
                .with_cache_dir(cache_dir)
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
            document_format: DocumentFormat::HatchdoorContextual,
            identity_suffix: "",
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

    pub fn nomic_v1_5_in(cache_dir: PathBuf) -> Result<Self, String> {
        Self::load_in(
            EmbeddingModel::NomicEmbedTextV15,
            768,
            1024,
            "NomicEmbedTextV15",
            "search_document: ",
            "search_query: ",
            cache_dir,
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

    /// GTE Base English v1.5 — native ONNX. English-only floor for the
    /// multilingual benchmark. No task-instruction prefixes. `max_length` is set
    /// to 1024 so the 800-token chunk sweep (plus context header) is not
    /// truncated.
    pub fn gte_base_en() -> Result<Self, String> {
        Self::load(
            EmbeddingModel::GTEBaseENV15,
            768,
            1024,
            "GTEBaseENV15",
            "",
            "",
        )
    }

    /// EmbeddingGemma 300M Q4 — multilingual 4-bit ONNX model. It has a
    /// 2,048-token input limit and supports Matryoshka truncation for the
    /// storage-efficient 256-dimensional evaluation variant.
    pub fn embedding_gemma_300m_q4() -> Result<Self, String> {
        Self::embedding_gemma_300m_q4_in(fastembed::get_cache_dir().into())
    }

    pub fn embedding_gemma_300m_q4_in(cache_dir: PathBuf) -> Result<Self, String> {
        let mut embedder = Self::load_in(
            EmbeddingModel::EmbeddingGemma300MQ4,
            768,
            2048,
            "EmbeddingGemma300MQ4",
            "",
            "task: search result | query: ",
            cache_dir,
        )?;
        // EmbeddingGemma's retrieval training uses different query and document
        // templates. This is intentionally a model-specific document format,
        // not merely a prefix, because the note title has its own field.
        embedder.document_format = DocumentFormat::GemmaRetrievalV1;
        // The cached vectors are incompatible with the earlier plain-context
        // Gemma experiment, even though the weights and output dimensions match.
        embedder.identity_suffix = "-gemma-retrieval-v1";
        Ok(embedder)
    }

    /// Wrap an already-constructed `TextEmbedding` (e.g. a user-defined ONNX
    /// model) so it reuses the same embed / token_count / identity path as the
    /// enum-based models.
    fn from_text_embedding(
        model: TextEmbedding,
        dim: usize,
        max_length: usize,
        id: &'static str,
        doc_prefix: &'static str,
        query_prefix: &'static str,
    ) -> Self {
        Self {
            model: Mutex::new(model),
            dim,
            max_length,
            id,
            doc_prefix,
            query_prefix,
            document_format: DocumentFormat::HatchdoorContextual,
            identity_suffix: "",
        }
    }

    /// Snowflake Arctic Embed M v2.0 — midsize multilingual, a retrieval
    /// fine-tune of gte-multilingual-base. Not a native FastEmbed enum model, so
    /// it is loaded as a user-defined ONNX model: the fp32 `onnx/model.onnx` with
    /// CLS pooling (per the repo's `1_Pooling/config.json`). The model card
    /// specifies a `"query: "` prefix for queries and no document prefix.
    /// Downloads ~1.2 GB on first use.
    pub fn arctic_m_v2() -> Result<Self, String> {
        const REPO: &str = "Snowflake/snowflake-arctic-embed-m-v2.0";
        const MAX_LENGTH: usize = 1024;

        let onnx_file = super::hub::fetch_bytes(REPO, "onnx/model.onnx")?;
        let tokenizer_files = TokenizerFiles {
            tokenizer_file: super::hub::fetch_bytes(REPO, "tokenizer.json")?,
            config_file: super::hub::fetch_bytes(REPO, "config.json")?,
            special_tokens_map_file: super::hub::fetch_bytes(REPO, "special_tokens_map.json")?,
            tokenizer_config_file: super::hub::fetch_bytes(REPO, "tokenizer_config.json")?,
        };
        let user_model =
            UserDefinedEmbeddingModel::new(onnx_file, tokenizer_files).with_pooling(Pooling::Cls);
        let model = TextEmbedding::try_new_from_user_defined(
            user_model,
            InitOptionsUserDefined::new().with_max_length(MAX_LENGTH),
        )
        .map_err(|e| format!("failed to load Arctic M v2.0 user-defined model: {e}"))?;

        Ok(Self::from_text_embedding(
            model,
            768,
            MAX_LENGTH,
            "SnowflakeArcticEmbedMV2",
            "",
            "query: ",
        ))
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
        format!(
            "{}{}",
            embedder_identity(self.id, self.dim, self.max_length),
            self.identity_suffix
        )
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

    fn document_input(&self, title: &str, heading_path: Option<&str>, body: &str) -> String {
        match self.document_format {
            DocumentFormat::HatchdoorContextual => format!(
                "{}{}",
                self.doc_prefix,
                crate::embed::contextual_document(title, heading_path, body)
            ),
            DocumentFormat::GemmaRetrievalV1 => gemma_retrieval_document(title, heading_path, body),
        }
    }
}

fn gemma_retrieval_document(title: &str, heading_path: Option<&str>, body: &str) -> String {
    let title = if title.trim().is_empty() {
        "none"
    } else {
        title
    };
    let text = match heading_path {
        Some(path) if !path.is_empty() => format!("Section: {path}\n\n{body}"),
        _ => body.to_string(),
    };
    format!("title: {title} | text: {text}")
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
    use super::{embedder_identity, gemma_retrieval_document};

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

    #[test]
    fn gemma_retrieval_document_uses_official_title_text_template() {
        assert_eq!(
            gemma_retrieval_document("Runbook", Some("Backups > Restore"), "Stop first."),
            "title: Runbook | text: Section: Backups > Restore\n\nStop first."
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
