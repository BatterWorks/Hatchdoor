//! Matryoshka dimension reduction as an embedder decorator. Wraps a full-size
//! model and truncates each vector to a shorter prefix, then re-normalises.
//! Because the cache derives its vector dimension, storage layout, and rebuild
//! trigger entirely from the embedder, this needs zero changes to the populate
//! path: `eval build --dim 256` produces a 256-dimensional index.

use std::sync::Arc;

use super::Embedder;

pub struct MatryoshkaEmbedder {
    inner: Arc<dyn Embedder>,
    dim: usize,
}

impl MatryoshkaEmbedder {
    /// Wrap `inner`, truncating its output to `dim`. Fails if `dim` is zero or
    /// larger than the model's native dimension (Matryoshka only shortens).
    pub fn new(inner: Arc<dyn Embedder>, dim: usize) -> Result<Self, String> {
        let full = inner.embedding_dim();
        if dim == 0 || dim > full {
            return Err(format!(
                "matryoshka dim {dim} must be in 1..={full} (the model's native dimension)"
            ));
        }
        Ok(Self { inner, dim })
    }
}

/// Truncate to the first `dim` components and re-normalise to unit length, as
/// Matryoshka-trained models expect for a reduced representation.
fn truncate_and_normalize(vector: &[f32], dim: usize) -> Vec<f32> {
    let mut out = vector[..dim].to_vec();
    let norm = out.iter().map(|x| x * x).sum::<f32>().sqrt().max(1e-12);
    for x in &mut out {
        *x /= norm;
    }
    out
}

impl Embedder for MatryoshkaEmbedder {
    fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, String> {
        let full = self.inner.embed(texts)?;
        Ok(full
            .iter()
            .map(|v| truncate_and_normalize(v, self.dim))
            .collect())
    }

    fn embedding_dim(&self) -> usize {
        self.dim
    }

    /// Distinct from the inner model so a cache built at a reduced dimension is
    /// never reused as if it held full-size vectors.
    fn identity(&self) -> String {
        format!("{}-mrl{}", self.inner.identity(), self.dim)
    }

    fn token_count(&self, text: &str, add_special_tokens: bool) -> Result<usize, String> {
        self.inner.token_count(text, add_special_tokens)
    }

    fn doc_prefix(&self) -> &'static str {
        self.inner.doc_prefix()
    }

    fn query_prefix(&self) -> &'static str {
        self.inner.query_prefix()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::embed::StubEmbedder;

    #[test]
    fn rejects_zero_and_oversized_dimensions() {
        let inner: Arc<dyn Embedder> = Arc::new(StubEmbedder::new(768));
        assert!(MatryoshkaEmbedder::new(inner.clone(), 0).is_err());
        assert!(MatryoshkaEmbedder::new(inner.clone(), 769).is_err());
        assert!(MatryoshkaEmbedder::new(inner, 256).is_ok());
    }

    #[test]
    fn reports_reduced_dimension_and_distinct_identity() {
        let inner: Arc<dyn Embedder> = Arc::new(StubEmbedder::new(768));
        let mrl = MatryoshkaEmbedder::new(inner.clone(), 256).expect("wrap");
        assert_eq!(mrl.embedding_dim(), 256);
        assert_ne!(mrl.identity(), inner.identity());
        assert!(mrl.identity().ends_with("-mrl256"));
    }

    #[test]
    fn embeds_truncated_renormalized_prefix_of_inner() {
        let inner: Arc<dyn Embedder> = Arc::new(StubEmbedder::new(768));
        let mrl = MatryoshkaEmbedder::new(inner.clone(), 256).expect("wrap");

        let text = vec!["hello world".to_string()];
        let reduced = mrl.embed(&text).expect("embed");
        assert_eq!(reduced.len(), 1);
        assert_eq!(reduced[0].len(), 256);

        let expected = truncate_and_normalize(&inner.embed(&text).expect("inner")[0], 256);
        assert_eq!(reduced[0], expected);

        let norm: f32 = reduced[0].iter().map(|x| x * x).sum::<f32>().sqrt();
        assert!(
            (norm - 1.0).abs() < 1e-5,
            "reduced vector must be unit length"
        );
    }
}
