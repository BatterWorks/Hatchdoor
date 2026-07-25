pub mod context;
pub mod embedder;
pub mod fastembed_embedder;

pub use context::contextual_document;
pub use embedder::{Embedder, StubEmbedder};
pub use fastembed_embedder::FastembedEmbedder;
