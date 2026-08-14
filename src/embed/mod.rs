pub mod candle_embedder;
pub mod context;
pub mod embedder;
pub mod fastembed_embedder;
pub mod hub;
pub mod matryoshka;

pub use candle_embedder::{NomicV2Embedder, Qwen3Embedder};
pub use context::contextual_document;
pub use embedder::{Embedder, PENDING_IDENTITY, RuntimeEmbedder, StubEmbedder};
pub use fastembed_embedder::FastembedEmbedder;
pub use matryoshka::MatryoshkaEmbedder;
