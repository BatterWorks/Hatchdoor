//! Read queries over the SQLite cache, split by concern. Every function is an
//! `impl SqliteCache` method, so callers are unaffected by the file layout.

mod graph;
mod metadata;
mod search;

// Only `SemanticHit` is named outside this module (re-exported from `cache`
// and used by the reranker). The other row types are reachable through the
// pub methods that return them, via type inference at the call sites.
pub use search::SemanticHit;
