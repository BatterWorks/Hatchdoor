use std::sync::Mutex;

use fastembed::{RerankInitOptions, RerankerModel, TextRerank};

use crate::cache::SemanticHit;

use super::reranker::{RerankedHit, Reranker};

/// fastembed-backed cross-encoder reranker.
pub struct FastembedReranker {
    // FastEmbed 5's `TextRerank::rerank` takes `&mut self` (Rayon dropped);
    // the `Reranker` trait is `&self` behind a shared `Arc`, so serialize
    // scoring through a Mutex.
    model: Mutex<TextRerank>,
    id: &'static str,
}

impl FastembedReranker {
    fn load(model: RerankerModel, id: &'static str) -> Result<Self, String> {
        let model =
            TextRerank::try_new(RerankInitOptions::new(model).with_show_download_progress(false))
                .map_err(|e| format!("failed to load reranker {id}: {e}"))?;
        Ok(Self {
            model: Mutex::new(model),
            id,
        })
    }

    pub fn id(&self) -> &'static str {
        self.id
    }

    pub fn jina_v1_turbo() -> Result<Self, String> {
        Self::load(
            RerankerModel::JINARerankerV1TurboEn,
            "JINARerankerV1TurboEn",
        )
    }

    /// NB: the fastembed enum variant misspells "Multilingual" as
    /// "Multiligual". We re-expose it under the corrected spelling
    /// for our own callers.
    pub fn jina_v2_multilingual() -> Result<Self, String> {
        Self::load(
            RerankerModel::JINARerankerV2BaseMultiligual,
            "JINARerankerV2BaseMultilingual",
        )
    }
}

impl Reranker for FastembedReranker {
    fn rerank(
        &self,
        query: &str,
        candidates: Vec<SemanticHit>,
    ) -> Result<Vec<RerankedHit>, String> {
        if candidates.is_empty() {
            return Ok(Vec::new());
        }

        let documents: Vec<&str> = candidates.iter().map(|c| c.content.as_str()).collect();
        let scored = self
            .model
            .lock()
            .map_err(|e| format!("reranker mutex poisoned: {e}"))?
            .rerank(query, documents, false, None)
            .map_err(|e| format!("rerank call failed: {e}"))?;

        let mut out: Vec<RerankedHit> = Vec::with_capacity(scored.len());
        for r in scored {
            let original = candidates
                .get(r.index)
                .ok_or_else(|| format!("reranker returned out-of-bounds index {}", r.index))?;
            out.push(RerankedHit {
                chunk_id: original.chunk_id,
                note_slug: original.note_slug.clone(),
                heading_path: original.heading_path.clone(),
                content: original.content.clone(),
                embedding_distance: original.distance,
                rerank_score: r.score,
            });
        }
        Ok(out)
    }

    fn id(&self) -> &'static str {
        self.id
    }
}

#[cfg(all(test, feature = "embedder-tests"))]
mod tests {
    use super::*;

    #[test]
    fn jina_v1_turbo_loads_and_ids_match() {
        let r = FastembedReranker::jina_v1_turbo().expect("load");
        assert_eq!(r.id(), "JINARerankerV1TurboEn");
    }

    #[test]
    fn jina_v2_multilingual_loads_and_ids_match() {
        let r = FastembedReranker::jina_v2_multilingual().expect("load");
        assert_eq!(r.id(), "JINARerankerV2BaseMultilingual");
    }

    #[test]
    fn jina_v1_turbo_reranks_two_candidates() {
        use crate::cache::SemanticHit;
        let r = FastembedReranker::jina_v1_turbo().expect("load");
        let cands = vec![
            SemanticHit {
                chunk_id: 1,
                note_slug: "off-topic".to_string(),
                heading_path: None,
                content: "I planted a fern in the corner of the kitchen.".to_string(),
                distance: 0.1,
            },
            SemanticHit {
                chunk_id: 2,
                note_slug: "on-topic".to_string(),
                heading_path: None,
                content: "Tips for flying with a baby on a plane.".to_string(),
                distance: 0.2,
            },
        ];
        let out = r
            .rerank("travelling by plane with the baby", cands)
            .expect("rerank");
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].note_slug, "on-topic");
    }
}
