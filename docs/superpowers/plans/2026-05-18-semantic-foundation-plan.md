# Phase 1 — Semantic Foundation Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add markdown-aware chunking, in-process embedding (Snowflake Arctic Embed S via fastembed-rs), and `sqlite-vec` vector storage to Hatchdoor so the existing SQLite cache also holds per-chunk 384-dim embeddings. Phase 1 is internal infrastructure; no new HTTP route or MCP tool ships.

**Architecture:** Two new modules — `src/chunk/` (pure markdown chunker on top of `text-splitter`) and `src/embed/` (owns the loaded `fastembed::TextEmbedding`, exposes an `Embedder` trait + a deterministic `StubEmbedder` for tests). Three touched modules — `cache/schema.rs` adds `chunks` + `chunk_vectors`; `cache/populate.rs` chunks and embeds each note in the same transaction that writes it; `cache/queries.rs` exposes `semantic_search`. The `sqlite-vec` extension registers itself via a static-init C call right after each `Connection::open`. Model weights are baked into the Docker image (final task) via a `--prefetch-embedder` startup flag invoked at build time.

**Tech Stack:** Rust 2024, Axum 0.8, `rusqlite` 0.39 (bundled SQLite), `fastembed` 4+, `sqlite-vec`, `text-splitter`, `tokenizers`, `blake3`, `tracing`. ONNX Runtime bundled by `fastembed`.

**Reference spec:** `docs/superpowers/specs/2026-05-18-semantic-foundation-design.md`. Read it once before starting; every task below maps to a section of it.

**Execution order:** Tasks 1–17 are local (cargo + your real vault on the host). Task 18 (Docker) is the last step before production. Do not jump to Docker mid-plan; iterate locally first.

---

## File map

| Path | Kind | Responsibility |
|---|---|---|
| `src/app_state.rs` | MODIFY | Task 1: retire `refresh_seconds`/`refresh_interval`/dead debounce path. |
| `src/handlers/api.rs` | MODIFY | Task 1: update `refresh_if_needed` call to drop the `force` arg. |
| `src/vault_watcher.rs` | MODIFY | Task 1: same. |
| `src/mcp/tools.rs` | MODIFY | Task 1: same (two call sites). |
| `src/mcp/routes.rs` | MODIFY | Task 1: remove `refresh_interval` from test fixtures. |
| `src/main.rs` | MODIFY | Task 1 (remove `refresh_interval` from AppState constructor) + Task 5 (CLI flag) + Task 14 (wire ArcticEmbedder). |
| `Dockerfile` | MODIFY | Task 1 (drop `VAULT_REFRESH_SECONDS`) + Task 18 (bake weights). |
| `.env.example` | MODIFY | Task 1: drop `VAULT_REFRESH_SECONDS`. |
| `Cargo.toml` | MODIFY | Task 2: add new deps. |
| `src/embed/mod.rs` | CREATE | Re-export `Embedder` trait, `ArcticEmbedder`, `StubEmbedder`. |
| `src/embed/embedder.rs` | CREATE | `Embedder` trait + `StubEmbedder` (deterministic, hash-based, used by all default tests). |
| `src/embed/arctic.rs` | CREATE | `ArcticEmbedder` — wraps `fastembed::TextEmbedding` + the matching `tokenizers::Tokenizer`. |
| `src/chunk/mod.rs` | CREATE | Re-export `chunk_note`, `Chunk`. |
| `src/chunk/chunker.rs` | CREATE | `chunk_note(content, tokenizer, opts) -> Vec<Chunk>` using `text-splitter`. Pure. |
| `src/chunk/normalize.rs` | CREATE | Frontmatter stripping, code-fence stripping, `tags`/`aliases` extraction. Pure. |
| `src/cache/schema.rs` | MODIFY | Bump schema version `2` → `3`, add `chunks` + `chunk_vectors`. |
| `src/cache/mod.rs` | MODIFY | Call `sqlite_vec::sqlite3_vec_init` after `Connection::open` in both `open` and `in_memory`. |
| `src/cache/populate.rs` | MODIFY | Inside the existing upsert transaction, call chunker + embedder + write chunks + vectors; add global orphan sweep at end of `replace_from_index`. |
| `src/cache/queries.rs` | MODIFY | Add `SqliteCache::semantic_search` + `SemanticHit` struct. |
| `src/cache/chunk_ops.rs` | CREATE | Helpers used by `populate.rs`: `replace_chunks_for_note`, `existing_chunk_hashes`, `delete_orphan_vectors`. |

`src/cache/parse.rs`, `src/vault/*` (except metadata flowing into chunks), `src/handlers/*` (except api.rs), and the rest of `src/mcp/*` are NOT touched in Phase 1.

---

## Conventions used in this plan

- **TDD is mandatory.** Every code-producing task starts with a failing test, then minimal implementation, then green test. AGENTS.md §3 requires regression tests for any production change.
- **Commit after each task.** Subject line follows AGENTS.md §7 (`type(scope): summary` + bullet body). Bodies are dictated per task below.
- **Test command for fast loop:** `cargo test --lib --no-fail-fast` (skips the real-model embedder tests, runs everything else).
- **Full test command:** `cargo fmt && cargo check && cargo clippy --all-targets -- -D warnings && cargo test --all-targets`.
- **Real-model tests:** gated behind `cfg(feature = "embedder-tests")`. Run with `cargo test --features embedder-tests`. Network access required the first time (Hugging Face download).
- **Branch:** work on `development`; do not merge to `main` from within these tasks.
- **No Docker until Task 18.** Local `cargo run --release` against `/home/battermanz/notes` is the verification target for tasks 1–17.

---

## Task 1: Retire `VAULT_REFRESH_SECONDS` and the dead debounce path

**Why first:** Every production caller of `refresh_if_needed` passes `force=true`. The `refresh_interval` field, the `refresh_seconds` config, the `VAULT_REFRESH_SECONDS` env var, and the `force=false` branch of `refresh_if_needed` are all dead code, kept alive only by the old polling path that no longer exists. Cleaning them up before Phase 1 starts means the new code doesn't inherit dead plumbing.

**Files:**
- Modify: `src/app_state.rs`
- Modify: `src/handlers/api.rs`
- Modify: `src/vault_watcher.rs`
- Modify: `src/mcp/tools.rs`
- Modify: `src/mcp/routes.rs`
- Modify: `src/main.rs`
- Modify: `Dockerfile`
- Modify: `.env.example`

- [ ] **Step 1: Confirm the dead-code premise**

Run: `grep -rn "refresh_if_needed(&state, false)\|refresh_if_needed(state, false)" src/`
Expected: zero hits in production code (test code may use `false`; those tests get removed below).

- [ ] **Step 2: Simplify `refresh_if_needed` and remove the config**

In `src/app_state.rs`:

- Remove `parse_refresh_seconds` and its two tests (`parse_refresh_seconds_accepts_valid_u64`, `parse_refresh_seconds_rejects_invalid_values`).
- Remove the `refresh_seconds: u64` field from `AppConfig` and the corresponding env read + parse in `from_env`.
- Remove the `refresh_interval: Duration` field from `AppState`.
- Remove the `last_refresh` field from `VaultCache`.
- Remove the `force: bool` argument from `refresh_if_needed`. New signature:

```rust
pub(crate) async fn refresh_if_needed(
    state: &AppState,
) -> Result<(), (StatusCode, Json<ErrorResponse>)> {
    let mut guard = state.cache.write().await;
    match build_cache_with_sqlite(&state.vault_path, guard.sqlite.clone()) {
        Ok(cache) => {
            info!(vault_path = %state.vault_path.display(), "SQLite vault cache refreshed");
            *guard = cache;
            broadcast_vault_revision(state);
            Ok(())
        }
        Err(error) => {
            error!(
                vault_path = %state.vault_path.display(),
                error = %error,
                "Vault refresh failed"
            );
            Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: format!("Vault refresh failed: {error}"),
                }),
            ))
        }
    }
}
```

- Remove `refresh_if_needed_skips_when_interval_not_elapsed` (the test exercises the path we just deleted).
- Rename `refresh_if_needed_force_refresh_surfaces_errors` to `refresh_if_needed_surfaces_errors`, drop the `false`/`true` argument from its call.
- Remove `refresh_interval` from the `state_with_vault` test helper signature; remove `refresh_interval` from every `AppState { ... }` literal.

- [ ] **Step 3: Update the four production call sites**

Change every call from `refresh_if_needed(&state, true)` (or `(state, true)`) to `refresh_if_needed(&state)` (or `(state)`):

- `src/handlers/api.rs:108`
- `src/vault_watcher.rs:48`
- `src/mcp/tools.rs:293`
- `src/mcp/tools.rs:569`

- [ ] **Step 4: Remove `refresh_interval` from test fixtures**

- `src/main.rs`: remove the `refresh_interval: Duration::from_secs(60)` line from the `AppState { ... }` literal inside `app_for_tests_with_state`. Remove the now-unused `Duration` import if it becomes unused.
- `src/mcp/routes.rs:174`: same removal in the test-only AppState constructor there.

- [ ] **Step 5: Remove the env var from runtime config**

- `Dockerfile`: in the `runtime` stage `ENV` block, delete the line `VAULT_REFRESH_SECONDS=2 \`. Leave the rest of the block exactly as-is.
- `.env.example`: delete the line `VAULT_REFRESH_SECONDS=2` and any explanatory comment immediately above it that only references this variable.

- [ ] **Step 6: Verify the build and tests**

Run: `cargo fmt && cargo check && cargo clippy --all-targets -- -D warnings && cargo test --lib`
Expected: clean. If clippy complains about unused imports (`Instant`, `Duration` in `app_state.rs`), remove them.

- [ ] **Step 7: Commit**

```bash
git add src/ Dockerfile .env.example
git commit -m "$(cat <<'EOF'
refactor(app): retire VAULT_REFRESH_SECONDS and the dead refresh-debounce path

- Drop refresh_seconds from AppConfig, refresh_interval from AppState, and last_refresh from VaultCache; the polling path that read them no longer exists.
- Simplify refresh_if_needed to take no force argument and always rebuild, matching what every production caller (watcher, /api/refresh, two MCP tools) was already passing.
- Remove VAULT_REFRESH_SECONDS from Dockerfile and .env.example.
- Delete the time-based debounce test and update the surviving tests to the new signature.
EOF
)"
```

---

## Task 2: Add Cargo dependencies

**Files:**
- Modify: `Cargo.toml`

- [ ] **Step 1: Add the new dependencies**

Edit `Cargo.toml` and append to `[dependencies]`:

```toml
fastembed = "4"
sqlite-vec = "0.1"
text-splitter = { version = "0.27", features = ["markdown", "tokenizers"] }
tokenizers = { version = "0.21", default-features = false, features = ["onig"] }
blake3 = "1"
bytemuck = "1"
```

Add a new `[features]` section if one does not exist:

```toml
[features]
default = []
embedder-tests = []
```

- [ ] **Step 2: Verify the build resolves**

Run: `cargo check`
Expected: success. The new crates compile (this also downloads `fastembed`'s ONNX runtime native library — first run takes a minute). If a version conflict appears, pin to the latest patch of the major shown above and re-run.

- [ ] **Step 3: Commit**

```bash
git add Cargo.toml Cargo.lock
git commit -m "$(cat <<'EOF'
feat(cache): add embedding and vector dependencies

- Add fastembed (in-process ONNX embeddings), sqlite-vec (vec0 virtual table via static init), text-splitter (markdown-aware token-accurate chunker), tokenizers (HuggingFace tokenizer for chunker/embedder pairing), blake3 (chunk content hashing), and bytemuck (safe &[f32]/&[u8] views for sqlite-vec binding).
- Add an embedder-tests feature flag to gate the real-model integration test, which requires Hugging Face access to download Arctic Embed S weights on first run.
EOF
)"
```

---

## Task 3: `Embedder` trait + `StubEmbedder`

**Files:**
- Create: `src/embed/mod.rs`
- Create: `src/embed/embedder.rs`
- Modify: `src/main.rs` (add `mod embed;`)

The trait is the entire public surface of the `embed` module. The stub is deterministic so integration tests can assert exact behaviour without loading a 130 MB model.

- [ ] **Step 1: Write failing tests for the stub**

Create `src/embed/embedder.rs`:

```rust
// Implementation below the tests.

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stub_embedder_produces_fixed_dim_vectors() {
        let embedder = StubEmbedder::new(384);
        let vectors = embedder.embed(&["hello".to_string(), "world".to_string()]).expect("embed");
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
```

Also create `src/embed/mod.rs`:

```rust
pub(crate) mod embedder;

pub(crate) use embedder::{Embedder, StubEmbedder};
```

Add `mod embed;` to `src/main.rs` near the other `mod` declarations.

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test --lib embed::embedder::tests`
Expected: compilation failure — `StubEmbedder`, `Embedder` trait, methods all undefined.

- [ ] **Step 3: Implement the trait and stub**

Insert above the `#[cfg(test)]` block in `src/embed/embedder.rs`:

```rust
use std::sync::Arc;

use tokenizers::{Tokenizer, models::wordlevel::WordLevel, pre_tokenizers::whitespace::Whitespace};

/// In-process text embedder. Loaded once at startup, shared via Arc.
pub(crate) trait Embedder: Send + Sync {
    /// Returns one embedding per input string, in order.
    fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, String>;

    /// Embedding dimensionality. Must be constant for the lifetime of the embedder.
    fn embedding_dim(&self) -> usize;

    /// The exact tokenizer the embedder uses internally, so the chunker can
    /// pre-compute token counts that match the embedder's accounting.
    fn tokenizer(&self) -> Arc<Tokenizer>;
}

/// Deterministic test embedder. Hashes each input to a fixed-dim vector so
/// tests can assert exact output without loading a real model.
pub(crate) struct StubEmbedder {
    dim: usize,
    tokenizer: Arc<Tokenizer>,
}

impl StubEmbedder {
    pub(crate) fn new(dim: usize) -> Self {
        let model = WordLevel::builder()
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

    fn embedding_dim(&self) -> usize { self.dim }

    fn tokenizer(&self) -> Arc<Tokenizer> { self.tokenizer.clone() }
}

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
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test --lib embed::embedder::tests`
Expected: five passing tests.

- [ ] **Step 5: Commit**

```bash
git add src/embed/ src/main.rs
git commit -m "$(cat <<'EOF'
feat(embed): introduce Embedder trait and deterministic StubEmbedder

- Add the Embedder trait covering embed(), embedding_dim(), and tokenizer(); held as Arc<dyn Embedder> by AppState in later tasks.
- Add StubEmbedder backed by BLAKE3-derived unit vectors so integration tests have deterministic, dim-correct embeddings without loading the real model.
- Use a whitespace WordLevel tokenizer for the stub, enough for tests that need a tokenizer-shaped object without real BPE behaviour.
EOF
)"
```

---

## Task 4: `ArcticEmbedder` (real fastembed wrapper)

**Files:**
- Create: `src/embed/arctic.rs`
- Modify: `src/embed/mod.rs`

The concrete embedder wraps `fastembed::TextEmbedding` and loads the matching tokenizer file separately so `text-splitter` can borrow it later. The real-model test is gated behind the `embedder-tests` feature so default `cargo test` stays fast.

For local development the model weights download to `~/.cache/fastembed/` on first use; Docker baking happens in Task 18.

- [ ] **Step 1: Write the failing real-model test**

Create `src/embed/arctic.rs`:

```rust
use std::sync::Arc;

use fastembed::{EmbeddingModel, InitOptions, TextEmbedding};
use tokenizers::Tokenizer;

use super::Embedder;

pub(crate) struct ArcticEmbedder {
    model: TextEmbedding,
    tokenizer: Arc<Tokenizer>,
    dim: usize,
}

const ARCTIC_S_DIM: usize = 384;

impl ArcticEmbedder {
    pub(crate) fn load() -> Result<Self, String> {
        let model = TextEmbedding::try_new(
            InitOptions::new(EmbeddingModel::SnowflakeArcticEmbedS).with_show_download_progress(false),
        )
        .map_err(|e| format!("failed to load Arctic Embed S: {e}"))?;

        let tokenizer = load_tokenizer()?;
        Ok(Self { model, tokenizer: Arc::new(tokenizer), dim: ARCTIC_S_DIM })
    }
}

impl Embedder for ArcticEmbedder {
    fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, String> {
        if texts.is_empty() { return Ok(Vec::new()); }
        self.model.embed(texts.to_vec(), None)
            .map_err(|e| format!("Arctic embed call failed: {e}"))
    }
    fn embedding_dim(&self) -> usize { self.dim }
    fn tokenizer(&self) -> Arc<Tokenizer> { self.tokenizer.clone() }
}

fn load_tokenizer() -> Result<Tokenizer, String> {
    use std::path::PathBuf;

    let cache_root = std::env::var("FASTEMBED_CACHE_PATH")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
            PathBuf::from(home).join(".cache").join("fastembed")
        });

    // fastembed serializes its model directories as `models--<org>--<name>/snapshots/<rev>/`.
    let arctic_root = cache_root.join("models--Snowflake--snowflake-arctic-embed-s");
    let snapshots = arctic_root.join("snapshots");
    let snapshot_dir = std::fs::read_dir(&snapshots)
        .map_err(|e| format!(
            "could not read fastembed snapshot dir {}: {e}. Was the model downloaded?",
            snapshots.display()
        ))?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .find(|p| p.is_dir())
        .ok_or_else(|| format!("no Arctic-S snapshot found under {}", snapshots.display()))?;

    let tokenizer_path = snapshot_dir.join("tokenizer.json");
    Tokenizer::from_file(&tokenizer_path)
        .map_err(|e| format!("failed to load tokenizer at {}: {e}", tokenizer_path.display()))
}

#[cfg(all(test, feature = "embedder-tests"))]
mod tests {
    use super::*;

    #[test]
    fn arctic_embedder_produces_384_dim_finite_vectors() {
        let embedder = ArcticEmbedder::load().expect("load Arctic-S");
        let vectors = embedder.embed(&["hello world".to_string(), "second input".to_string()]).expect("embed");
        assert_eq!(vectors.len(), 2);
        assert_eq!(vectors[0].len(), 384);
        assert_eq!(vectors[1].len(), 384);
        assert!(vectors[0].iter().all(|v| v.is_finite()));
        assert_eq!(embedder.embedding_dim(), 384);
    }

    #[test]
    fn arctic_embedder_is_deterministic_for_identical_input() {
        let embedder = ArcticEmbedder::load().expect("load Arctic-S");
        let a = embedder.embed(&["hello".to_string()]).expect("first");
        let b = embedder.embed(&["hello".to_string()]).expect("second");
        assert_eq!(a, b);
    }

    #[test]
    fn arctic_tokenizer_is_loaded_alongside_model() {
        let embedder = ArcticEmbedder::load().expect("load Arctic-S");
        let encoding = embedder.tokenizer().encode("hello world", false).expect("encode");
        assert!(!encoding.get_ids().is_empty());
    }
}
```

Add to `src/embed/mod.rs`:

```rust
pub(crate) mod arctic;
pub(crate) use arctic::ArcticEmbedder;
```

- [ ] **Step 2: Run the test to verify it fails (first time only)**

Run: `cargo test --features embedder-tests --lib embed::arctic`
Expected on a *fresh* machine: first failure is a download taking 30–60 s, then the tests should pass once the cache exists. If the test fails because `load_tokenizer` cannot find `tokenizer.json`, inspect `~/.cache/fastembed/` to confirm the directory name `models--Snowflake--snowflake-arctic-embed-s` matches what fastembed actually wrote and adjust the constant.

- [ ] **Step 3: Run the default test suite**

Run: `cargo test --lib`
Expected: every prior test still passes; the Arctic tests are skipped (feature off).

- [ ] **Step 4: Commit**

```bash
git add src/embed/arctic.rs src/embed/mod.rs
git commit -m "$(cat <<'EOF'
feat(embed): add ArcticEmbedder backed by fastembed and matching tokenizer

- Wrap fastembed::TextEmbedding for SnowflakeArcticEmbedS (384-dim) behind the Embedder trait.
- Load the model's tokenizer.json directly from the fastembed cache directory so text-splitter can share the exact tokenizer the model uses, keeping token counts consistent across chunker and embedder.
- Gate the real-model integration test behind the embedder-tests feature so default cargo test stays offline and fast.
EOF
)"
```

---

## Task 5: `--prefetch-embedder` CLI flag

**Files:**
- Modify: `src/main.rs`

Bake-at-build (Task 18) needs a way to trigger the model download from inside the Docker build stage. A single command-line flag that constructs an `ArcticEmbedder` and exits is enough. Also useful locally: lets you warm the embedder cache without spinning up the server.

- [ ] **Step 1: Write a failing test for the flag parser**

Add to the existing `tests` module in `src/main.rs`:

```rust
#[test]
fn cli_recognises_prefetch_embedder_flag() {
    let args = vec!["hatchdoor".to_string(), "--prefetch-embedder".to_string()];
    assert!(matches!(parse_run_mode(&args), RunMode::PrefetchEmbedder));
}

#[test]
fn cli_defaults_to_serve_mode() {
    let args = vec!["hatchdoor".to_string()];
    assert!(matches!(parse_run_mode(&args), RunMode::Serve));
}

#[test]
fn cli_rejects_unknown_flags() {
    let args = vec!["hatchdoor".to_string(), "--bogus".to_string()];
    assert!(matches!(parse_run_mode(&args), RunMode::Unknown(_)));
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test --lib tests::cli_`
Expected: compilation failure — `parse_run_mode`, `RunMode` undefined.

- [ ] **Step 3: Implement the parser and dispatch**

Above `fn main` in `src/main.rs`:

```rust
enum RunMode {
    Serve,
    PrefetchEmbedder,
    Unknown(String),
}

fn parse_run_mode(args: &[String]) -> RunMode {
    match args.get(1).map(String::as_str) {
        None => RunMode::Serve,
        Some("--prefetch-embedder") => RunMode::PrefetchEmbedder,
        Some(other) => RunMode::Unknown(other.to_string()),
    }
}
```

Then change `fn main` to dispatch:

```rust
#[tokio::main]
async fn main() {
    dotenv().ok();
    init_logging();

    let args: Vec<String> = std::env::args().collect();
    match parse_run_mode(&args) {
        RunMode::Serve => run_server().await,
        RunMode::PrefetchEmbedder => run_prefetch(),
        RunMode::Unknown(flag) => {
            error!("Unknown flag: {flag}");
            std::process::exit(2);
        }
    }
}

fn run_prefetch() {
    use crate::embed::ArcticEmbedder;
    info!("Pre-fetching Arctic Embed S weights and tokenizer");
    match ArcticEmbedder::load() {
        Ok(_) => info!("Pre-fetch complete"),
        Err(e) => { error!("Pre-fetch failed: {e}"); std::process::exit(1); }
    }
}
```

Move the existing body of `fn main` into a new `async fn run_server()`. Keep all imports as they are.

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test --lib tests::cli_`
Expected: three passing tests.

- [ ] **Step 5: Commit**

```bash
git add src/main.rs
git commit -m "$(cat <<'EOF'
feat(cli): add --prefetch-embedder flag for warming the embedder cache

- Parse argv into a RunMode enum (Serve, PrefetchEmbedder, Unknown) so the same binary can either start the HTTP server or pre-download the Arctic Embed S weights and exit.
- Extract the original main body into run_server() and add a run_prefetch() entry point used both locally (cache warmup) and by the Dockerfile in Task 18.
- Cover the parser with three unit tests so flag dispatch is regression-tested.
EOF
)"
```

---

## Task 6: Schema bump + `chunks` and `chunk_vectors` tables

**Files:**
- Modify: `src/cache/schema.rs`

The current schema version is `"2"`. We bump to `"3"` and add the two new tables. The existing schema-version check rejects unknown versions outright (forcing a delete-and-rebuild), so callers running against an old cache file will see a clear error.

- [ ] **Step 1: Write a failing test for the schema bump**

Add to the bottom of `src/cache/schema.rs`:

```rust
#[cfg(test)]
mod tests {
    use crate::cache::SqliteCache;

    #[test]
    fn fresh_cache_creates_chunks_and_chunk_vectors_tables() {
        let cache = SqliteCache::in_memory().expect("open");
        let conn = cache.connection().expect("conn");

        let chunks: i64 = conn.query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'chunks'",
            [], |row| row.get(0)).expect("query");
        assert_eq!(chunks, 1, "chunks table must exist");

        let chunk_vectors: i64 = conn.query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE name = 'chunk_vectors'",
            [], |row| row.get(0)).expect("query");
        assert_eq!(chunk_vectors, 1, "chunk_vectors virtual table must exist");
    }

    #[test]
    fn fresh_cache_records_schema_version_3() {
        let cache = SqliteCache::in_memory().expect("open");
        let conn = cache.connection().expect("conn");
        let version: String = conn.query_row(
            "SELECT value FROM metadata WHERE key = 'schema_version'",
            [], |row| row.get(0)).expect("query");
        assert_eq!(version, "3");
    }
}
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test --lib cache::schema::tests`
Expected: failure — `chunk_vectors` not created (will error at `SqliteCache::in_memory()` once we add the `CREATE VIRTUAL TABLE` statement because `vec0` is not registered yet). That is the signal to proceed to Task 7 immediately after.

- [ ] **Step 3: Update the schema**

In `src/cache/schema.rs`, change:

```rust
const SCHEMA_VERSION: &str = "2";
```

to:

```rust
const SCHEMA_VERSION: &str = "3";
```

Inside the `create_schema` SQL block, append before the final `INSERT INTO metadata` statement:

```sql
CREATE TABLE IF NOT EXISTS chunks (
    id           INTEGER PRIMARY KEY AUTOINCREMENT,
    note_slug    TEXT    NOT NULL REFERENCES notes(slug) ON DELETE CASCADE ON UPDATE CASCADE,
    ordinal      INTEGER NOT NULL,
    heading_path TEXT,
    content      TEXT    NOT NULL,
    byte_start   INTEGER NOT NULL,
    byte_end     INTEGER NOT NULL,
    content_hash TEXT    NOT NULL,
    tags         TEXT,
    aliases      TEXT
);

CREATE INDEX IF NOT EXISTS idx_chunks_note_slug ON chunks(note_slug);
CREATE INDEX IF NOT EXISTS idx_chunks_content_hash ON chunks(content_hash);

CREATE VIRTUAL TABLE IF NOT EXISTS chunk_vectors USING vec0(
    chunk_id  INTEGER PRIMARY KEY,
    embedding FLOAT[384]
);
```

Update the version literal inside the `INSERT INTO metadata` statement from `'2'` to `'3'`.

- [ ] **Step 4: Run the tests**

Run: `cargo test --lib cache::schema::tests`
Expected at this point: still failing — `vec0` is not yet registered. Move directly to Task 7.

- [ ] **Step 5: Commit**

```bash
git add src/cache/schema.rs
git commit -m "$(cat <<'EOF'
feat(cache): add chunks and chunk_vectors tables, bump schema to v3

- Add a chunks table holding per-note retrieval units with ordinal, heading_path, byte range, BLAKE3 content hash, and lifted tags/aliases JSON, keyed by id with note_slug as a cascading FK.
- Add a vec0 virtual table chunk_vectors keyed by chunk_id storing 384-dim Arctic Embed S embeddings.
- Bump schema_version to 3 so older caches force a clean rebuild rather than silently mismatching dim/columns.
- Schema tests assert both tables and the new version land in a fresh in-memory cache; tests fail until sqlite-vec init lands in the next task.
EOF
)"
```

---

## Task 7: Wire `sqlite-vec` static init at connection open

**Files:**
- Modify: `src/cache/mod.rs`

- [ ] **Step 1: Modify `SqliteCache::open` and `in_memory`**

In `src/cache/mod.rs`, both constructors must call `sqlite_vec::sqlite3_vec_init` before `ensure_schema`:

```rust
impl SqliteCache {
    pub(crate) fn open(path: impl AsRef<Path>) -> Result<Self, String> {
        let path = path.as_ref();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|error| {
                format!("failed to create SQLite cache directory '{}': {error}", parent.display())
            })?;
        }

        let conn = Connection::open(path)
            .map_err(|error| format!("failed to open SQLite cache '{}': {error}", path.display()))?;
        register_sqlite_vec(&conn)?;
        let cache = Self { conn: Mutex::new(conn) };
        cache.ensure_schema()?;
        Ok(cache)
    }

    #[cfg(test)]
    pub(crate) fn in_memory() -> Result<Self, String> {
        let conn = Connection::open_in_memory()
            .map_err(|error| format!("failed to open in-memory SQLite cache: {error}"))?;
        register_sqlite_vec(&conn)?;
        let cache = Self { conn: Mutex::new(conn) };
        cache.ensure_schema()?;
        Ok(cache)
    }

    pub(crate) fn connection(&self) -> Result<MutexGuard<'_, Connection>, String> {
        self.conn.lock().map_err(|_| "SQLite cache connection lock poisoned".to_string())
    }
}

fn register_sqlite_vec(conn: &Connection) -> Result<(), String> {
    // Safety: registering a SQLite extension via its C entry point on a freshly
    // opened connection is sound. The returned rc is checked below.
    let rc = unsafe { sqlite_vec::sqlite3_vec_init(conn.handle()) };
    if rc != 0 {
        return Err(format!("sqlite-vec init failed with code {rc}"));
    }
    Ok(())
}
```

`Connection::handle()` is available with the project's existing `bundled` feature.

- [ ] **Step 2: Run the schema tests**

Run: `cargo test --lib cache::schema::tests`
Expected: both tests from Task 6 now pass.

- [ ] **Step 3: Run the full library test suite**

Run: `cargo test --lib`
Expected: every existing test still passes.

- [ ] **Step 4: Commit**

```bash
git add src/cache/mod.rs
git commit -m "$(cat <<'EOF'
feat(cache): register sqlite-vec extension at connection open

- Call sqlite_vec::sqlite3_vec_init on every Connection (both on-disk and in-memory) before ensure_schema, so vec0 virtual tables can be created without load_extension or shipping a separate .so file.
- Single unsafe block encapsulated in register_sqlite_vec; treat any non-zero return as a startup error.
- Turns the schema tests from Task 6 green now that the vec0 module is registered.
EOF
)"
```

---

## Task 8: Pre-embed normalization (`src/chunk/normalize.rs`)

**Files:**
- Create: `src/chunk/mod.rs`
- Create: `src/chunk/normalize.rs`
- Modify: `src/main.rs` (add `mod chunk;`)

Pure functions, no IO.

- [ ] **Step 1: Write failing tests for normalization**

Create `src/chunk/normalize.rs`:

```rust
// Implementation below the tests.

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strip_frontmatter_removes_yaml_block_at_start() {
        let input = "---\ntitle: Foo\ntags: [a, b]\n---\n\n# Heading\n\nBody.";
        assert_eq!(strip_frontmatter(input), "# Heading\n\nBody.");
    }

    #[test]
    fn strip_frontmatter_leaves_content_without_frontmatter_untouched() {
        let input = "# Heading\n\nBody.";
        assert_eq!(strip_frontmatter(input), input);
    }

    #[test]
    fn strip_frontmatter_ignores_yaml_block_not_at_start() {
        let input = "# Heading\n\n---\nnot frontmatter\n---";
        assert_eq!(strip_frontmatter(input), input);
    }

    #[test]
    fn strip_code_fences_removes_fence_lines_keeps_contents() {
        let input = "before\n```rust\nfn foo() {}\n```\nafter";
        assert_eq!(strip_code_fences(input), "before\nfn foo() {}\nafter");
    }

    #[test]
    fn extract_tags_and_aliases_pulls_from_yaml_frontmatter() {
        let input = "---\ntags: [project, hatchdoor]\naliases:\n  - hd\n  - door\n---\nbody";
        let meta = extract_frontmatter_metadata(input);
        assert_eq!(meta.tags, vec!["project", "hatchdoor"]);
        assert_eq!(meta.aliases, vec!["hd", "door"]);
    }

    #[test]
    fn extract_tags_and_aliases_returns_empty_for_no_frontmatter() {
        let meta = extract_frontmatter_metadata("just body");
        assert!(meta.tags.is_empty());
        assert!(meta.aliases.is_empty());
    }
}
```

Create `src/chunk/mod.rs`:

```rust
pub(crate) mod normalize;
```

Add `mod chunk;` to `src/main.rs` near the other module declarations.

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test --lib chunk::normalize::tests`
Expected: compilation failure — `strip_frontmatter`, `strip_code_fences`, `extract_frontmatter_metadata`, `FrontmatterMetadata` undefined.

- [ ] **Step 3: Implement normalization**

Insert above the `#[cfg(test)]` block in `src/chunk/normalize.rs`:

```rust
pub(crate) struct FrontmatterMetadata {
    pub tags: Vec<String>,
    pub aliases: Vec<String>,
}

pub(crate) fn strip_frontmatter(content: &str) -> &str {
    if !content.starts_with("---") { return content; }
    let after_open = match content.strip_prefix("---") {
        Some(rest) => rest,
        None => return content,
    };
    let after_open = after_open.trim_start_matches(['\r']);
    let after_open = match after_open.strip_prefix('\n') {
        Some(rest) => rest,
        None => return content,
    };
    let mut search_from = 0;
    while let Some(idx) = after_open[search_from..].find("\n---") {
        let abs = search_from + idx + 1;
        let end_marker = &after_open[abs..];
        let after_marker = end_marker.strip_prefix("---").unwrap_or(end_marker);
        let after_marker = after_marker.trim_start_matches(['\r']);
        if after_marker.is_empty() || after_marker.starts_with('\n') {
            return after_marker.trim_start_matches(['\r', '\n']);
        }
        search_from = abs + 3;
    }
    content
}

pub(crate) fn strip_code_fences(content: &str) -> String {
    let mut out = String::with_capacity(content.len());
    for line in content.split_inclusive('\n') {
        if line.trim_start().starts_with("```") { continue; }
        out.push_str(line);
    }
    out
}

pub(crate) fn extract_frontmatter_metadata(content: &str) -> FrontmatterMetadata {
    let mut tags = Vec::new();
    let mut aliases = Vec::new();
    if !content.starts_with("---") {
        return FrontmatterMetadata { tags, aliases };
    }
    let after_open = content.strip_prefix("---").unwrap_or("").trim_start_matches(['\r', '\n']);
    let end = match after_open.find("\n---") {
        Some(idx) => idx,
        None => return FrontmatterMetadata { tags, aliases },
    };
    let block = &after_open[..end];
    parse_simple_yaml_list(block, "tags", &mut tags);
    parse_simple_yaml_list(block, "aliases", &mut aliases);
    FrontmatterMetadata { tags, aliases }
}

fn parse_simple_yaml_list(block: &str, key: &str, out: &mut Vec<String>) {
    let mut lines = block.lines().peekable();
    while let Some(line) = lines.next() {
        let trimmed = line.trim_end();
        let stripped = match trimmed.strip_prefix(key) {
            Some(rest) => rest,
            None => continue,
        };
        let rest = stripped.trim_start();
        if !rest.starts_with(':') { continue; }
        let value = rest[1..].trim();
        if value.starts_with('[') && value.ends_with(']') {
            let inner = &value[1..value.len() - 1];
            for item in inner.split(',') {
                let item = item.trim().trim_matches(|c| c == '"' || c == '\'');
                if !item.is_empty() { out.push(item.to_string()); }
            }
            return;
        }
        if value.is_empty() {
            while let Some(next) = lines.peek() {
                let next_trim = next.trim_start();
                if let Some(item) = next_trim.strip_prefix("- ") {
                    out.push(item.trim().trim_matches(|c| c == '"' || c == '\'').to_string());
                    lines.next();
                } else { break; }
            }
            return;
        }
        out.push(value.trim_matches(|c| c == '"' || c == '\'').to_string());
        return;
    }
}
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test --lib chunk::normalize::tests`
Expected: six tests pass.

- [ ] **Step 5: Commit**

```bash
git add src/chunk/ src/main.rs
git commit -m "$(cat <<'EOF'
feat(chunk): add pure normalization helpers for pre-embed content cleanup

- Add strip_frontmatter that removes a leading YAML block delimited by --- markers.
- Add strip_code_fences that drops ``` fence lines while keeping the code contents so identifier semantics are preserved in embeddings.
- Add extract_frontmatter_metadata returning tags and aliases lifted from frontmatter, ready to be stored on chunks as JSON arrays.
- Six unit tests cover the common YAML shapes (inline list, block list, single value, missing block) and frontmatter false-positives.
EOF
)"
```

---

## Task 9: Chunker (`src/chunk/chunker.rs`)

**Files:**
- Create: `src/chunk/chunker.rs`
- Modify: `src/chunk/mod.rs`

- [ ] **Step 1: Write failing tests for the chunker**

Create `src/chunk/chunker.rs`:

```rust
use std::sync::Arc;

use tokenizers::Tokenizer;

use super::normalize::{extract_frontmatter_metadata, strip_code_fences, strip_frontmatter};

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Chunk {
    pub ordinal: usize,
    pub heading_path: Option<String>,
    pub content: String,
    pub byte_start: usize,
    pub byte_end: usize,
    pub content_hash: String,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct ChunkOptions {
    pub max_tokens: usize,
    pub overlap_tokens: usize,
}

impl Default for ChunkOptions {
    fn default() -> Self { Self { max_tokens: 800, overlap_tokens: 50 } }
}

pub(crate) struct NoteChunking {
    pub chunks: Vec<Chunk>,
    pub tags: Vec<String>,
    pub aliases: Vec<String>,
}

pub(crate) fn chunk_note(
    raw_content: &str,
    tokenizer: Arc<Tokenizer>,
    opts: ChunkOptions,
) -> NoteChunking {
    todo!()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::embed::{Embedder, StubEmbedder};

    fn stub_tokenizer() -> Arc<Tokenizer> {
        StubEmbedder::new(384).tokenizer()
    }

    #[test]
    fn empty_input_produces_no_chunks() {
        let result = chunk_note("", stub_tokenizer(), ChunkOptions::default());
        assert!(result.chunks.is_empty());
    }

    #[test]
    fn small_single_section_produces_one_chunk() {
        let content = "# Heading\n\nA short paragraph.";
        let result = chunk_note(content, stub_tokenizer(), ChunkOptions::default());
        assert_eq!(result.chunks.len(), 1);
        assert_eq!(result.chunks[0].ordinal, 0);
        assert!(result.chunks[0].content.contains("short paragraph"));
    }

    #[test]
    fn chunks_have_deterministic_blake3_hashes() {
        let content = "# A\n\nbody";
        let a = chunk_note(content, stub_tokenizer(), ChunkOptions::default());
        let b = chunk_note(content, stub_tokenizer(), ChunkOptions::default());
        assert_eq!(a.chunks, b.chunks);
        assert_eq!(a.chunks[0].content_hash.len(), 64);
    }

    #[test]
    fn ordinals_are_sequential_from_zero() {
        let content = "# A\nfirst\n\n# B\nsecond\n\n# C\nthird";
        let result = chunk_note(content, stub_tokenizer(), ChunkOptions { max_tokens: 5, overlap_tokens: 0 });
        for (i, chunk) in result.chunks.iter().enumerate() {
            assert_eq!(chunk.ordinal, i);
        }
        assert!(result.chunks.len() >= 3);
    }

    #[test]
    fn heading_path_reflects_nested_headings() {
        let content = "# Top\n\n## Sub\n\ndeep body";
        let result = chunk_note(content, stub_tokenizer(), ChunkOptions::default());
        let last = result.chunks.last().expect("chunk");
        let path = last.heading_path.as_deref().unwrap_or("");
        assert!(path.contains("Top") || path.contains("Sub"));
    }

    #[test]
    fn frontmatter_is_stripped_before_chunking() {
        let content = "---\ntags: [x, y]\n---\n\n# A\n\nbody";
        let result = chunk_note(content, stub_tokenizer(), ChunkOptions::default());
        assert!(result.chunks.iter().all(|c| !c.content.contains("tags: [x, y]")));
        assert_eq!(result.tags, vec!["x", "y"]);
    }

    #[test]
    fn code_fences_are_stripped_but_code_contents_remain() {
        let content = "# A\n\n```rust\nfn foo() {}\n```\n";
        let result = chunk_note(content, stub_tokenizer(), ChunkOptions::default());
        let joined: String = result.chunks.iter().map(|c| c.content.clone()).collect();
        assert!(joined.contains("fn foo()"));
        assert!(!joined.contains("```"));
    }

    #[test]
    fn wikilinks_are_preserved_literally() {
        let content = "# A\n\nsee [[Other Note]] for context";
        let result = chunk_note(content, stub_tokenizer(), ChunkOptions::default());
        let joined: String = result.chunks.iter().map(|c| c.content.clone()).collect();
        assert!(joined.contains("[[Other Note]]"));
    }

    #[test]
    fn oversized_section_is_split_under_max_tokens() {
        let big = "para. ".repeat(2_000);
        let content = format!("# A\n\n{big}");
        let opts = ChunkOptions { max_tokens: 50, overlap_tokens: 5 };
        let result = chunk_note(&content, stub_tokenizer(), opts);
        let tokenizer = stub_tokenizer();
        for chunk in &result.chunks {
            let encoding = tokenizer.encode(chunk.content.as_str(), false).expect("encode");
            assert!(encoding.get_ids().len() <= opts.max_tokens + opts.overlap_tokens);
        }
    }
}
```

Add to `src/chunk/mod.rs`:

```rust
pub(crate) mod chunker;
pub(crate) use chunker::{Chunk, ChunkOptions, NoteChunking, chunk_note};
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test --lib chunk::chunker::tests`
Expected: nine tests, all failing with the `todo!()`.

- [ ] **Step 3: Implement `chunk_note`**

Replace the `todo!()` body with:

```rust
pub(crate) fn chunk_note(
    raw_content: &str,
    tokenizer: Arc<Tokenizer>,
    opts: ChunkOptions,
) -> NoteChunking {
    use text_splitter::{ChunkConfig, MarkdownSplitter};

    let metadata = extract_frontmatter_metadata(raw_content);
    let body = strip_frontmatter(raw_content);
    let normalized = strip_code_fences(body);

    if normalized.trim().is_empty() {
        return NoteChunking {
            chunks: Vec::new(),
            tags: metadata.tags,
            aliases: metadata.aliases,
        };
    }

    let config = ChunkConfig::new(opts.max_tokens)
        .with_sizer((*tokenizer).clone())
        .with_overlap(opts.overlap_tokens)
        .expect("overlap must be < max_tokens");
    let splitter = MarkdownSplitter::new(config);

    let mut chunks = Vec::new();
    for (ordinal, (byte_start, piece)) in splitter.chunk_indices(&normalized).enumerate() {
        let byte_end = byte_start + piece.len();
        let heading_path = derive_heading_path(&normalized, byte_start);
        let content = piece.to_string();
        let content_hash = blake3::hash(content.as_bytes()).to_hex().to_string();
        chunks.push(Chunk { ordinal, heading_path, content, byte_start, byte_end, content_hash });
    }

    NoteChunking { chunks, tags: metadata.tags, aliases: metadata.aliases }
}

/// Walk all ATX headings (`#`, `##`, `###`) at or before `byte_offset` and
/// reconstruct the heading stack as `"H1 > H2 > H3"`.
fn derive_heading_path(content: &str, byte_offset: usize) -> Option<String> {
    let prefix = &content[..byte_offset.min(content.len())];
    let mut stack: [Option<String>; 3] = [None, None, None];
    for line in prefix.lines() {
        let trimmed = line.trim_start();
        let level = trimmed.chars().take_while(|c| *c == '#').count();
        if level == 0 || level > 3 { continue; }
        let text = trimmed[level..].trim();
        if text.is_empty() { continue; }
        stack[level - 1] = Some(text.to_string());
        for deeper in &mut stack[level..] { *deeper = None; }
    }
    let parts: Vec<String> = stack.iter().filter_map(Clone::clone).collect();
    if parts.is_empty() { None } else { Some(parts.join(" > ")) }
}
```

If the installed `text-splitter` API differs, run `cargo doc --open --no-deps -p text-splitter` and adapt — the *shape* (tokenizer-based sizer, max tokens, overlap, markdown-aware splitter, `chunk_indices` for byte offsets) is what matters.

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test --lib chunk::chunker::tests`
Expected: all nine tests pass.

- [ ] **Step 5: Commit**

```bash
git add src/chunk/
git commit -m "$(cat <<'EOF'
feat(chunk): add markdown chunker built on text-splitter

- Wrap text-splitter::MarkdownSplitter with the embedder's tokenizer so chunk size accounting matches the embedder's token budget exactly.
- Apply the spec normalization pipeline (strip frontmatter, strip code fences, keep wikilinks literal) before splitting, and lift tags/aliases as separate metadata.
- Reconstruct the heading stack for each chunk by scanning ATX headings up to the chunk's byte offset, yielding "H1 > H2 > H3" paths consumers can show alongside hits.
- Hash each chunk's content with BLAKE3 so populate.rs can short-circuit re-embedding for unchanged chunks.
- Nine unit tests cover small/large notes, headings, frontmatter, code fences, wikilinks, ordinal sequencing, deterministic hashes, and the oversized-section sub-split guarantee.
EOF
)"
```

---

## Task 10: Chunk-ops helpers (`src/cache/chunk_ops.rs`)

**Files:**
- Create: `src/cache/chunk_ops.rs`
- Modify: `src/cache/mod.rs` (add `mod chunk_ops;`)

Keeping chunk persistence in its own helper module avoids growing `populate.rs` further (AGENTS.md §1.6) and isolates the SQL that touches `chunks` and `chunk_vectors`.

- [ ] **Step 1: Write failing tests for the helpers**

Create `src/cache/chunk_ops.rs`:

```rust
use rusqlite::{Transaction, params};

use crate::chunk::Chunk;

pub(crate) struct ChunkRow<'a> {
    pub chunk: &'a Chunk,
    pub vector: &'a [f32],
}

pub(crate) fn replace_chunks_for_note(
    tx: &Transaction<'_>,
    note_slug: &str,
    rows: &[ChunkRow<'_>],
    tags_json: Option<&str>,
    aliases_json: Option<&str>,
) -> Result<(), String> { todo!() }

pub(crate) fn existing_chunk_hashes(
    tx: &Transaction<'_>,
    note_slug: &str,
) -> Result<std::collections::HashMap<String, i64>, String> { todo!() }

pub(crate) fn delete_orphan_vectors(tx: &Transaction<'_>) -> Result<usize, String> { todo!() }

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cache::SqliteCache;

    fn fake_chunk(ordinal: usize, content: &str) -> Chunk {
        Chunk {
            ordinal,
            heading_path: Some("H".to_string()),
            content: content.to_string(),
            byte_start: 0,
            byte_end: content.len(),
            content_hash: blake3::hash(content.as_bytes()).to_hex().to_string(),
        }
    }

    fn insert_minimal_note(cache: &SqliteCache, slug: &str) {
        let conn = cache.connection().expect("conn");
        conn.execute(
            r#"INSERT INTO notes (slug, title, normalized_title, relative_path,
                normalized_relative_path, absolute_path, content, content_hash,
                mtime_ns, size_bytes, indexed_at)
               VALUES (?, 'T', 't', ?, ?, '/abs', 'c', 'h', 0, 0, 0)"#,
            params![slug, format!("{slug}.md"), format!("{slug}.md")],
        ).expect("insert note");
    }

    #[test]
    fn replace_chunks_inserts_new_chunks_and_vectors() {
        let cache = SqliteCache::in_memory().expect("open");
        insert_minimal_note(&cache, "n1");
        let chunk = fake_chunk(0, "hello");
        let vector = vec![0.1f32; 384];

        let mut conn = cache.connection().expect("conn");
        let tx = conn.transaction().expect("tx");
        replace_chunks_for_note(&tx, "n1",
            &[ChunkRow { chunk: &chunk, vector: &vector }], None, None).expect("replace");
        tx.commit().expect("commit");

        let conn = cache.connection().expect("conn");
        let count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM chunks WHERE note_slug = 'n1'", [], |r| r.get(0)).expect("count");
        assert_eq!(count, 1);
        let vec_count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM chunk_vectors", [], |r| r.get(0)).expect("count");
        assert_eq!(vec_count, 1);
    }

    #[test]
    fn replace_chunks_drops_previous_chunks_and_vectors_for_note() {
        let cache = SqliteCache::in_memory().expect("open");
        insert_minimal_note(&cache, "n1");
        let vector = vec![0.1f32; 384];

        let mut conn = cache.connection().expect("conn");
        let tx = conn.transaction().expect("tx");
        replace_chunks_for_note(&tx, "n1",
            &[ChunkRow { chunk: &fake_chunk(0, "old"), vector: &vector }], None, None).expect("write");
        tx.commit().expect("commit");

        let mut conn = cache.connection().expect("conn");
        let tx = conn.transaction().expect("tx");
        replace_chunks_for_note(&tx, "n1", &[
            ChunkRow { chunk: &fake_chunk(0, "fresh-1"), vector: &vector },
            ChunkRow { chunk: &fake_chunk(1, "fresh-2"), vector: &vector },
        ], None, None).expect("rewrite");
        tx.commit().expect("commit");

        let conn = cache.connection().expect("conn");
        let count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM chunks WHERE note_slug = 'n1'", [], |r| r.get(0)).expect("count");
        assert_eq!(count, 2);
        let vec_count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM chunk_vectors", [], |r| r.get(0)).expect("count");
        assert_eq!(vec_count, 2);
    }

    #[test]
    fn existing_chunk_hashes_returns_hash_to_id_map() {
        let cache = SqliteCache::in_memory().expect("open");
        insert_minimal_note(&cache, "n1");
        let chunk = fake_chunk(0, "hello");
        let vector = vec![0.1f32; 384];

        let mut conn = cache.connection().expect("conn");
        let tx = conn.transaction().expect("tx");
        replace_chunks_for_note(&tx, "n1",
            &[ChunkRow { chunk: &chunk, vector: &vector }], None, None).expect("write");

        let map = existing_chunk_hashes(&tx, "n1").expect("read");
        assert_eq!(map.len(), 1);
        assert!(map.contains_key(&chunk.content_hash));
    }

    #[test]
    fn delete_orphan_vectors_removes_vectors_without_chunks() {
        let cache = SqliteCache::in_memory().expect("open");
        let mut conn = cache.connection().expect("conn");
        let tx = conn.transaction().expect("tx");
        let vec_bytes = bytemuck::cast_slice(&vec![0.1f32; 384]).to_vec();
        tx.execute("INSERT INTO chunk_vectors (chunk_id, embedding) VALUES (?, ?)",
            params![9999i64, vec_bytes]).expect("insert orphan");
        let removed = delete_orphan_vectors(&tx).expect("sweep");
        assert_eq!(removed, 1);
        let count: i64 = tx.query_row(
            "SELECT COUNT(*) FROM chunk_vectors", [], |r| r.get(0)).expect("count");
        assert_eq!(count, 0);
    }
}
```

Add `mod chunk_ops;` to `src/cache/mod.rs`.

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test --lib cache::chunk_ops::tests`
Expected: four tests, all failing with `todo!()`.

- [ ] **Step 3: Implement the helpers**

Replace the bodies in `src/cache/chunk_ops.rs`:

```rust
pub(crate) fn replace_chunks_for_note(
    tx: &Transaction<'_>,
    note_slug: &str,
    rows: &[ChunkRow<'_>],
    tags_json: Option<&str>,
    aliases_json: Option<&str>,
) -> Result<(), String> {
    tx.execute(
        "DELETE FROM chunk_vectors WHERE chunk_id IN (SELECT id FROM chunks WHERE note_slug = ?1)",
        params![note_slug],
    ).map_err(|e| format!("failed to clear chunk_vectors for {note_slug}: {e}"))?;
    tx.execute("DELETE FROM chunks WHERE note_slug = ?1", params![note_slug])
        .map_err(|e| format!("failed to clear chunks for {note_slug}: {e}"))?;

    if rows.is_empty() { return Ok(()); }

    let mut insert_chunk = tx.prepare(
        r#"INSERT INTO chunks
           (note_slug, ordinal, heading_path, content, byte_start, byte_end, content_hash, tags, aliases)
           VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
           RETURNING id"#,
    ).map_err(|e| format!("prepare chunk insert: {e}"))?;
    let mut insert_vector = tx.prepare(
        "INSERT INTO chunk_vectors (chunk_id, embedding) VALUES (?1, ?2)",
    ).map_err(|e| format!("prepare vector insert: {e}"))?;

    for row in rows {
        let chunk_id: i64 = insert_chunk.query_row(
            params![
                note_slug,
                row.chunk.ordinal as i64,
                row.chunk.heading_path,
                row.chunk.content,
                row.chunk.byte_start as i64,
                row.chunk.byte_end as i64,
                row.chunk.content_hash,
                tags_json,
                aliases_json,
            ],
            |r| r.get(0),
        ).map_err(|e| format!("insert chunk: {e}"))?;
        let vector_bytes: &[u8] = bytemuck::cast_slice(row.vector);
        insert_vector.execute(params![chunk_id, vector_bytes])
            .map_err(|e| format!("insert vector: {e}"))?;
    }
    Ok(())
}

pub(crate) fn existing_chunk_hashes(
    tx: &Transaction<'_>,
    note_slug: &str,
) -> Result<std::collections::HashMap<String, i64>, String> {
    let mut stmt = tx.prepare(
        "SELECT content_hash, id FROM chunks WHERE note_slug = ?1"
    ).map_err(|e| format!("prepare hash query: {e}"))?;
    let rows = stmt.query_map(params![note_slug], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
    }).map_err(|e| format!("query chunk hashes: {e}"))?;
    let mut map = std::collections::HashMap::new();
    for row in rows {
        let (hash, id) = row.map_err(|e| format!("read chunk hash row: {e}"))?;
        map.insert(hash, id);
    }
    Ok(map)
}

pub(crate) fn delete_orphan_vectors(tx: &Transaction<'_>) -> Result<usize, String> {
    let removed = tx.execute(
        "DELETE FROM chunk_vectors WHERE chunk_id NOT IN (SELECT id FROM chunks)", [],
    ).map_err(|e| format!("delete orphan vectors: {e}"))?;
    Ok(removed)
}
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test --lib cache::chunk_ops::tests`
Expected: four passing tests.

- [ ] **Step 5: Commit**

```bash
git add src/cache/chunk_ops.rs src/cache/mod.rs
git commit -m "$(cat <<'EOF'
feat(cache): add chunk_ops helpers for chunk and vector persistence

- Introduce replace_chunks_for_note that atomically wipes a note's prior chunks and vectors before inserting new ones, keeping populate.rs free of new SQL.
- Add existing_chunk_hashes returning content_hash -> chunk_id so callers can short-circuit re-embedding for unchanged chunks.
- Add delete_orphan_vectors as the global sweep at the end of replace_from_index, removing any chunk_vectors rows whose chunk has disappeared.
EOF
)"
```

---

## Task 11: Wire chunking and embedding into `populate.rs`

**Files:**
- Modify: `src/cache/populate.rs`

The current `upsert_note_if_changed` returns `Result<(), String>` and short-circuits when the note is unchanged. We change it to return `Result<UpsertOutcome, String>` so callers can tell "wrote a new/updated row" apart from "no change", and only do chunk+embed work in the first case.

- [ ] **Step 1: Write a failing integration test**

Add to the bottom of `src/cache/populate.rs`:

```rust
#[cfg(test)]
mod chunk_integration_tests {
    use std::sync::Arc;

    use tempfile::TempDir;

    use crate::cache::SqliteCache;
    use crate::embed::{Embedder, StubEmbedder};
    use crate::vault::VaultIndex;

    fn make_vault(files: &[(&str, &str)]) -> TempDir {
        let dir = TempDir::new().expect("tempdir");
        for (name, body) in files {
            std::fs::write(dir.path().join(name), body).expect("write");
        }
        dir
    }

    #[test]
    fn replace_from_index_chunks_and_embeds_every_note() {
        let dir = make_vault(&[("a.md", "# A\n\nbody A"), ("b.md", "# B\n\nbody B")]);
        let cache = SqliteCache::in_memory().expect("open");
        let embedder: Arc<dyn Embedder> = Arc::new(StubEmbedder::new(384));
        let index = VaultIndex::build(&dir.path().to_path_buf()).expect("build");

        cache.replace_from_index_with_embedder(&index, embedder.as_ref()).expect("replace");

        let conn = cache.connection().expect("conn");
        let note_count: i64 = conn.query_row("SELECT COUNT(*) FROM notes", [], |r| r.get(0)).expect("count");
        assert_eq!(note_count, 2);
        let chunk_count: i64 = conn.query_row("SELECT COUNT(*) FROM chunks", [], |r| r.get(0)).expect("count");
        assert!(chunk_count >= 2);
        let vector_count: i64 = conn.query_row("SELECT COUNT(*) FROM chunk_vectors", [], |r| r.get(0)).expect("count");
        assert_eq!(vector_count, chunk_count);
    }

    #[test]
    fn unchanged_note_triggers_zero_new_embedding_calls() {
        struct CountingEmbedder {
            inner: StubEmbedder,
            calls: std::sync::atomic::AtomicUsize,
        }
        impl Embedder for CountingEmbedder {
            fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, String> {
                self.calls.fetch_add(texts.len(), std::sync::atomic::Ordering::SeqCst);
                self.inner.embed(texts)
            }
            fn embedding_dim(&self) -> usize { self.inner.embedding_dim() }
            fn tokenizer(&self) -> std::sync::Arc<tokenizers::Tokenizer> { self.inner.tokenizer() }
        }

        let dir = make_vault(&[("a.md", "# A\n\nbody A")]);
        let cache = SqliteCache::in_memory().expect("open");
        let embedder = Arc::new(CountingEmbedder { inner: StubEmbedder::new(384), calls: 0.into() });

        let index = VaultIndex::build(&dir.path().to_path_buf()).expect("build");
        cache.replace_from_index_with_embedder(&index, embedder.as_ref()).expect("first");
        let first_calls = embedder.calls.load(std::sync::atomic::Ordering::SeqCst);
        assert!(first_calls >= 1);

        cache.replace_from_index_with_embedder(&index, embedder.as_ref()).expect("second");
        let second_calls = embedder.calls.load(std::sync::atomic::Ordering::SeqCst);
        assert_eq!(second_calls, first_calls, "unchanged note must not re-embed");
    }

    #[test]
    fn deleting_a_note_removes_its_chunks_and_vectors() {
        let dir = make_vault(&[("a.md", "# A\n\nbody A"), ("b.md", "# B\n\nbody B")]);
        let cache = SqliteCache::in_memory().expect("open");
        let embedder: Arc<dyn Embedder> = Arc::new(StubEmbedder::new(384));

        let index1 = VaultIndex::build(&dir.path().to_path_buf()).expect("build1");
        cache.replace_from_index_with_embedder(&index1, embedder.as_ref()).expect("first");

        std::fs::remove_file(dir.path().join("b.md")).expect("remove");
        let index2 = VaultIndex::build(&dir.path().to_path_buf()).expect("build2");
        cache.replace_from_index_with_embedder(&index2, embedder.as_ref()).expect("second");

        let conn = cache.connection().expect("conn");
        let chunks_for_b: i64 = conn.query_row(
            "SELECT COUNT(*) FROM chunks WHERE note_slug = 'b'", [], |r| r.get(0)).expect("count");
        assert_eq!(chunks_for_b, 0);
        let total_vectors: i64 = conn.query_row("SELECT COUNT(*) FROM chunk_vectors", [], |r| r.get(0)).expect("count");
        let total_chunks: i64 = conn.query_row("SELECT COUNT(*) FROM chunks", [], |r| r.get(0)).expect("count");
        assert_eq!(total_vectors, total_chunks, "no orphan vectors after delete");
    }
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test --lib cache::populate::chunk_integration_tests`
Expected: compilation failure — `replace_from_index_with_embedder` not defined.

- [ ] **Step 3: Implement the embedder-aware path**

In `src/cache/populate.rs`:

1. Add imports:

```rust
use crate::cache::chunk_ops::{ChunkRow, delete_orphan_vectors, existing_chunk_hashes, replace_chunks_for_note};
use crate::chunk::{ChunkOptions, chunk_note};
use crate::embed::Embedder;
```

2. Define an outcome enum next to `upsert_note_if_changed`:

```rust
pub(crate) enum UpsertOutcome {
    Wrote { slug: String },
    Unchanged,
}
```

3. Change the return type of `upsert_note_if_changed` to `Result<UpsertOutcome, String>`. Audit every existing return site:
   - The "row was inserted or updated" paths return `Ok(UpsertOutcome::Wrote { slug: entry.slug() })` (use whichever existing slug source is in scope — `entry.slug.clone()` or the slug computed inside the function).
   - The "no change" path returns `Ok(UpsertOutcome::Unchanged)`.

4. Add the embedder-aware orchestrator method right after the existing `replace_from_index` (keep `replace_from_index` in place so legacy tests stay green until Task 12):

```rust
impl SqliteCache {
    pub(crate) fn replace_from_index_with_embedder(
        &self,
        index: &VaultIndex,
        embedder: &dyn Embedder,
    ) -> Result<(), String> {
        let entries = index.ordered_entries();
        let current_paths = entries
            .iter()
            .map(|entry| entry.relative_path.clone())
            .collect::<HashSet<_>>();
        let now = current_unix_timestamp();
        let mut conn = self.connection()?;
        let tx = conn.transaction()
            .map_err(|e| format!("failed to start SQLite cache refresh: {e}"))?;

        for cached_path in cached_relative_paths(&tx)? {
            if !current_paths.contains(&cached_path) {
                delete_note_by_relative_path(&tx, &cached_path)?;
            }
        }

        for entry in &entries {
            if let UpsertOutcome::Wrote { slug } = upsert_note_if_changed(&tx, entry, now)? {
                chunk_and_embed_note(&tx, &slug, entry, embedder)?;
            }
        }

        rebuild_links(&tx, index, &entries)?;
        let removed = delete_orphan_vectors(&tx)?;
        if removed > 0 {
            tracing::debug!(removed, "Swept orphan chunk vectors");
        }

        tx.commit().map_err(|e| format!("failed to commit SQLite cache refresh: {e}"))?;
        Ok(())
    }
}

pub(crate) struct ChunkStats {
    pub embedded: usize,
    pub reused: usize,
}

fn chunk_and_embed_note(
    tx: &Transaction<'_>,
    slug: &str,
    entry: &NoteEntry,
    embedder: &dyn Embedder,
) -> Result<ChunkStats, String> {
    let tokenizer = embedder.tokenizer();
    let chunking = chunk_note(&entry.content, tokenizer, ChunkOptions::default());
    if chunking.chunks.is_empty() {
        replace_chunks_for_note(tx, slug, &[], None, None)?;
        return Ok(ChunkStats { embedded: 0, reused: 0 });
    }

    let existing = existing_chunk_hashes(tx, slug)?;
    let preserved = preserve_existing_vectors(tx, slug, &chunking.chunks, &existing)?;

    let mut texts_to_embed: Vec<String> = Vec::new();
    let mut indices_needing_embed: Vec<usize> = Vec::new();
    for (idx, chunk) in chunking.chunks.iter().enumerate() {
        if !preserved.contains_key(&chunk.content_hash) {
            texts_to_embed.push(chunk.content.clone());
            indices_needing_embed.push(idx);
        }
    }

    let new_vectors = if texts_to_embed.is_empty() {
        Vec::new()
    } else {
        embedder.embed(&texts_to_embed)?
    };

    let mut vectors: Vec<Vec<f32>> = Vec::with_capacity(chunking.chunks.len());
    let mut new_iter = new_vectors.into_iter();
    let mut need_new: std::collections::HashSet<usize> = indices_needing_embed.iter().copied().collect();
    for (idx, chunk) in chunking.chunks.iter().enumerate() {
        if need_new.remove(&idx) {
            vectors.push(new_iter.next().ok_or("embedder returned too few vectors")?);
        } else {
            vectors.push(
                preserved.get(&chunk.content_hash)
                    .cloned()
                    .ok_or("preserved vector missing for unchanged chunk")?,
            );
        }
    }

    let tags_json = serde_json::to_string(&chunking.tags).ok();
    let aliases_json = serde_json::to_string(&chunking.aliases).ok();
    let rows: Vec<ChunkRow<'_>> = chunking.chunks.iter().zip(vectors.iter())
        .map(|(chunk, vector)| ChunkRow { chunk, vector }).collect();

    replace_chunks_for_note(tx, slug, &rows, tags_json.as_deref(), aliases_json.as_deref())?;
    Ok(ChunkStats {
        embedded: indices_needing_embed.len(),
        reused: chunking.chunks.len() - indices_needing_embed.len(),
    })
}

fn preserve_existing_vectors(
    tx: &Transaction<'_>,
    _slug: &str,
    chunks: &[crate::chunk::Chunk],
    existing: &std::collections::HashMap<String, i64>,
) -> Result<std::collections::HashMap<String, Vec<f32>>, String> {
    let mut out = std::collections::HashMap::new();
    let mut stmt = tx.prepare("SELECT embedding FROM chunk_vectors WHERE chunk_id = ?1")
        .map_err(|e| format!("prepare vector lookup: {e}"))?;
    for chunk in chunks {
        if let Some(chunk_id) = existing.get(&chunk.content_hash) {
            let bytes: Vec<u8> = stmt.query_row(rusqlite::params![chunk_id], |row| row.get(0))
                .map_err(|e| format!("read preserved vector: {e}"))?;
            let floats: Vec<f32> = bytemuck::cast_slice(&bytes).to_vec();
            out.insert(chunk.content_hash.clone(), floats);
        }
    }
    Ok(out)
}
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test --lib cache::populate`
Expected: the three new tests pass; all prior populate tests still pass.

- [ ] **Step 5: Commit**

```bash
git add src/cache/populate.rs
git commit -m "$(cat <<'EOF'
feat(cache): chunk and embed notes inside the populate transaction

- Add SqliteCache::replace_from_index_with_embedder mirroring the existing replace_from_index but invoking the chunker, the embedder, and chunk_ops persistence for every note that changed.
- Change upsert_note_if_changed to return an UpsertOutcome enum so the orchestrator can skip chunk+embed work entirely when a note is unchanged.
- Reuse existing chunk vectors via content_hash so single-paragraph edits in a 20-chunk note trigger one embedder call, not twenty.
- Add the spec's global orphan-vector sweep at the end of the transaction (no-op when no notes were deleted, microseconds otherwise).
- Three new integration tests using StubEmbedder + a CountingEmbedder cover happy path, the unchanged-note hash-skip regression, and delete propagation.
EOF
)"
```

---

## Task 12: `SqliteCache::semantic_search`

**Files:**
- Modify: `src/cache/queries.rs`

Spec §11.3. Internal-only, used by tests and the Phase 1.5 eval harness.

- [ ] **Step 1: Write a failing test**

Add to the bottom of `src/cache/queries.rs`:

```rust
#[cfg(test)]
mod semantic_search_tests {
    use std::sync::Arc;

    use tempfile::TempDir;

    use crate::cache::SqliteCache;
    use crate::embed::{Embedder, StubEmbedder};
    use crate::vault::VaultIndex;

    fn vault_with(files: &[(&str, &str)]) -> TempDir {
        let dir = TempDir::new().expect("tempdir");
        for (name, body) in files {
            std::fs::write(dir.path().join(name), body).expect("write");
        }
        dir
    }

    #[test]
    fn semantic_search_returns_hits_ordered_by_distance() {
        let dir = vault_with(&[
            ("a.md", "# Apples\n\napples and oranges"),
            ("b.md", "# Bicycles\n\nspokes and wheels"),
        ]);
        let cache = SqliteCache::in_memory().expect("open");
        let embedder: Arc<dyn Embedder> = Arc::new(StubEmbedder::new(384));
        let index = VaultIndex::build(&dir.path().to_path_buf()).expect("build");
        cache.replace_from_index_with_embedder(&index, embedder.as_ref()).expect("index");

        let hits = cache.semantic_search(embedder.as_ref(), "apples and oranges", 5).expect("search");
        assert!(!hits.is_empty());
        for w in hits.windows(2) {
            assert!(w[0].distance <= w[1].distance);
        }
    }

    #[test]
    fn semantic_search_respects_limit() {
        let dir = vault_with(&[
            ("a.md", "# A\n\nfirst"),
            ("b.md", "# B\n\nsecond"),
            ("c.md", "# C\n\nthird"),
        ]);
        let cache = SqliteCache::in_memory().expect("open");
        let embedder: Arc<dyn Embedder> = Arc::new(StubEmbedder::new(384));
        let index = VaultIndex::build(&dir.path().to_path_buf()).expect("build");
        cache.replace_from_index_with_embedder(&index, embedder.as_ref()).expect("index");

        let hits = cache.semantic_search(embedder.as_ref(), "anything", 2).expect("search");
        assert_eq!(hits.len(), 2);
    }

    #[test]
    fn semantic_search_returns_empty_when_no_chunks() {
        let cache = SqliteCache::in_memory().expect("open");
        let embedder: Arc<dyn Embedder> = Arc::new(StubEmbedder::new(384));
        let hits = cache.semantic_search(embedder.as_ref(), "anything", 5).expect("search");
        assert!(hits.is_empty());
    }
}
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test --lib cache::queries::semantic_search_tests`
Expected: compilation failure — `SemanticHit`, `semantic_search` undefined.

- [ ] **Step 3: Implement the query**

Add to `src/cache/queries.rs`:

```rust
use crate::embed::Embedder;

#[derive(Debug, Clone)]
pub(crate) struct SemanticHit {
    pub chunk_id: i64,
    pub note_slug: String,
    pub heading_path: Option<String>,
    pub content: String,
    pub distance: f32,
}

impl SqliteCache {
    pub(crate) fn semantic_search(
        &self,
        embedder: &dyn Embedder,
        query: &str,
        k: usize,
    ) -> Result<Vec<SemanticHit>, String> {
        let query_vec = embedder.embed(&[query.to_string()])?
            .into_iter().next().ok_or("embedder returned no vectors")?;
        let query_bytes: &[u8] = bytemuck::cast_slice(&query_vec);

        let conn = self.connection()?;
        let mut stmt = conn.prepare(
            r#"
            SELECT v.chunk_id, c.note_slug, c.heading_path, c.content, v.distance
            FROM chunk_vectors v
            JOIN chunks c ON c.id = v.chunk_id
            WHERE v.embedding MATCH ?1
            ORDER BY v.distance
            LIMIT ?2
            "#,
        ).map_err(|e| format!("prepare semantic_search: {e}"))?;
        let rows = stmt.query_map(rusqlite::params![query_bytes, k as i64], |row| {
            Ok(SemanticHit {
                chunk_id: row.get(0)?,
                note_slug: row.get(1)?,
                heading_path: row.get(2)?,
                content: row.get(3)?,
                distance: row.get::<_, f64>(4)? as f32,
            })
        }).map_err(|e| format!("query semantic_search: {e}"))?;
        let mut hits = Vec::new();
        for row in rows {
            hits.push(row.map_err(|e| format!("read semantic_search row: {e}"))?);
        }
        Ok(hits)
    }
}
```

If `sqlite-vec` reports a different distance column name in your installed version, run `cargo doc --open --no-deps -p sqlite-vec` and adjust the SELECT.

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test --lib cache::queries::semantic_search_tests`
Expected: three passing tests.

- [ ] **Step 5: Commit**

```bash
git add src/cache/queries.rs
git commit -m "$(cat <<'EOF'
feat(cache): add internal SqliteCache::semantic_search for Phase 1.5 eval

- Embed the query string through the supplied Embedder, then run a vec0 MATCH against chunk_vectors joined with chunks to return SemanticHit rows ordered by distance.
- Keep the method module-private; no HTTP route, no MCP tool. Phase 2 hybrid retrieval will build on top.
- Three new tests cover ordering by distance, the k limit, and the empty-cache case.
EOF
)"
```

---

## Task 13: `AppState` holds an `Embedder`

**Files:**
- Modify: `src/app_state.rs`

- [ ] **Step 1: Modify `AppState` and constructors**

In `src/app_state.rs`:

```rust
use crate::embed::Embedder;

#[derive(Clone)]
pub(crate) struct AppState {
    pub(crate) vault_path: PathBuf,
    pub(crate) cache: Arc<RwLock<VaultCache>>,
    pub(crate) vault_revision: Arc<AtomicU64>,
    pub(crate) vault_events: broadcast::Sender<u64>,
    pub(crate) embedder: Arc<dyn Embedder>,
}
```

Update `build_cache_with_sqlite` to take an embedder and route through the new orchestrator:

```rust
pub(crate) fn build_cache_with_sqlite(
    vault_path: &PathBuf,
    sqlite: Arc<SqliteCache>,
    embedder: &dyn Embedder,
) -> Result<VaultCache, String> {
    debug!(vault_path = %vault_path.display(), "Building SQLite vault cache");
    let index = VaultIndex::build(vault_path).map_err(|e| e.to_string())?;
    sqlite.replace_from_index_with_embedder(&index, embedder)?;
    Ok(VaultCache { sqlite })
}
```

Update `refresh_if_needed` to pass `state.embedder.as_ref()` into `build_cache_with_sqlite`. Update the `#[cfg(test)] build_cache` helper to take an embedder argument too.

- [ ] **Step 2: Update every test caller**

Every test that constructs `AppState` or calls `build_cache` must:

- Construct `Arc::new(StubEmbedder::new(384)) as Arc<dyn Embedder>`.
- Pass it to `build_cache(...)` / `build_cache_with_sqlite(...)`.
- Add `embedder: <that arc>` to the `AppState { ... }` literal.

Search for sites:

```bash
grep -rn "AppState {" src/
grep -rn "build_cache(" src/
grep -rn "build_cache_with_sqlite(" src/
```

Add a single helper next to `state_with_vault`:

```rust
#[cfg(test)]
pub(crate) fn test_embedder() -> Arc<dyn Embedder> {
    Arc::new(crate::embed::StubEmbedder::new(384))
}
```

- [ ] **Step 3: Run the full test suite**

Run: `cargo test --lib`
Expected: all tests pass.

- [ ] **Step 4: Commit**

```bash
git add src/
git commit -m "$(cat <<'EOF'
feat(state): thread Arc<dyn Embedder> through AppState and cache builds

- Add embedder: Arc<dyn Embedder> to AppState, passed in at startup and reused by every refresh.
- Change build_cache_with_sqlite and the test-only build_cache to take the embedder, and route refresh_if_needed through replace_from_index_with_embedder.
- Update every test constructor of AppState to inject a StubEmbedder so default cargo test stays offline.
EOF
)"
```

---

## Task 14: Construct `ArcticEmbedder` at startup and wire everything together

**Files:**
- Modify: `src/main.rs`

- [ ] **Step 1: Wire the embedder into `run_server`**

In `src/main.rs`, modify `run_server` so it builds an `ArcticEmbedder` before opening the cache, then passes it into `AppState`:

```rust
async fn run_server() {
    let config = AppConfig::from_env().unwrap_or_else(|e| {
        error!("Configuration error: {e}");
        std::process::exit(1);
    });

    let sqlite = Arc::new(
        SqliteCache::open(&config.cache_db_path).unwrap_or_else(|e| {
            error!(cache_db_path = %config.cache_db_path.display(), "SQLite cache startup failed: {e}");
            std::process::exit(1);
        }),
    );

    let embedder: Arc<dyn embed::Embedder> = Arc::new(
        embed::ArcticEmbedder::load().unwrap_or_else(|e| {
            error!("Embedder load failed: {e}");
            std::process::exit(1);
        }),
    );

    let cache = build_cache_with_sqlite(&config.vault_path, sqlite, embedder.as_ref())
        .unwrap_or_else(|e| {
            error!(
                "Failed to index vault at {} into SQLite cache {}: {e}",
                config.vault_path.display(),
                config.cache_db_path.display()
            );
            std::process::exit(1);
        });

    let (vault_events, _) = tokio::sync::broadcast::channel(64);
    let state = AppState {
        vault_path: config.vault_path.clone(),
        cache: Arc::new(RwLock::new(cache)),
        vault_revision: Arc::new(std::sync::atomic::AtomicU64::new(0)),
        vault_events,
        embedder,
    };

    vault_watcher::spawn_vault_watcher(
        state.clone(),
        config.vault_path.clone(),
        config.cache_db_path.clone(),
    );

    let app = build_router(state);

    let addr = config.socket_addr().unwrap_or_else(|e| {
        error!("Address error: {e}");
        std::process::exit(1);
    });

    info!(
        host = %config.host,
        port = config.port,
        vault_path = %config.vault_path.display(),
        cache_db_path = %config.cache_db_path.display(),
        "Hatchdoor starting"
    );
    info!("Hatchdoor listening on http://{addr}");
    let listener = tokio::net::TcpListener::bind(addr).await.unwrap_or_else(|e| {
        error!("Failed to bind: {e}");
        std::process::exit(1);
    });

    axum::serve(listener, app).await.unwrap_or_else(|e| {
        error!("Server error: {e}");
        std::process::exit(1);
    });
}
```

- [ ] **Step 2: Build to verify**

Run: `cargo build --release`
Expected: success.

- [ ] **Step 3: Run library tests once more**

Run: `cargo test --lib`
Expected: clean.

- [ ] **Step 4: Commit**

```bash
git add src/main.rs
git commit -m "$(cat <<'EOF'
feat(server): construct ArcticEmbedder at startup and inject into AppState

- Load Arctic Embed S once after opening the SQLite cache; exit non-zero on failure so the operator gets a loud signal instead of a silently-disabled embedder.
- Pass the embedder into build_cache_with_sqlite and into AppState so refresh_if_needed and any future call site reuses the same instance.
EOF
)"
```

---

## Task 15: Indexing observability

**Files:**
- Modify: `src/cache/populate.rs`

- [ ] **Step 1: Add the logging**

In `replace_from_index_with_embedder`, before the per-entry loop:

```rust
let started_at = std::time::Instant::now();
tracing::info!(notes = entries.len(), "Indexing vault: chunking and embedding");
let mut chunks_embedded: usize = 0;
let mut chunks_reused: usize = 0;
let mut per_note_failures: usize = 0;
```

Wrap the `chunk_and_embed_note` call:

```rust
for entry in &entries {
    if let UpsertOutcome::Wrote { slug } = upsert_note_if_changed(&tx, entry, now)? {
        match chunk_and_embed_note(&tx, &slug, entry, embedder) {
            Ok(stats) => {
                chunks_embedded += stats.embedded;
                chunks_reused += stats.reused;
            }
            Err(e) => {
                per_note_failures += 1;
                tracing::warn!(slug = %slug, error = %e, "Per-note embedding failed; skipped");
            }
        }
    }
}
```

After the loop, the closing log:

```rust
tracing::info!(
    notes = entries.len(),
    chunks_embedded,
    chunks_reused,
    per_note_failures,
    elapsed_ms = started_at.elapsed().as_millis(),
    "Indexing complete"
);
```

Add a `DEBUG` line inside `chunk_and_embed_note` just before `embedder.embed`:

```rust
if !texts_to_embed.is_empty() {
    tracing::debug!(
        slug,
        new = texts_to_embed.len(),
        reused = chunking.chunks.len() - texts_to_embed.len(),
        "Embedding chunks for note"
    );
}
```

- [ ] **Step 2: Run the tests**

Run: `cargo test --lib`
Expected: all tests pass.

- [ ] **Step 3: Commit**

```bash
git add src/cache/populate.rs
git commit -m "$(cat <<'EOF'
feat(cache): log indexing progress and per-note failures

- Emit INFO at indexing start/end with note count, embedded/reused chunk totals, per-note failure count, and elapsed milliseconds so a ~60s cold start does not look like a hang.
- Emit DEBUG per note when chunks are sent to the embedder (off by default; enabled via RUST_LOG=hatchdoor=debug).
- Emit WARN per per-note failure; the surrounding transaction is per-note, so a failed embed rolls back that note without poisoning the run.
EOF
)"
```

---

## Task 16: Local end-to-end verification against the real vault

**Files:** none modified — verification only. Run from the project root.

This is the gate before Docker. Treat any failure as a blocker.

- [ ] **Step 1: Warm the embedder cache**

Run: `cargo run --release -- --prefetch-embedder`
Expected: logs "Pre-fetching Arctic Embed S weights and tokenizer" then "Pre-fetch complete". First run downloads ~130 MB; subsequent runs return immediately.

- [ ] **Step 2: Point at the real vault and start the server**

In one terminal:

```bash
VAULT_PATH=/home/battermanz/notes \
HATCHDOOR_CACHE_DB=./tmp/hatchdoor-cache.sqlite3 \
RUST_LOG=hatchdoor=info \
cargo run --release
```

Expected log lines, in order:
- `Indexing vault: chunking and embedding notes=286`
- `Indexing complete notes=286 chunks_embedded=… chunks_reused=0 per_note_failures=0 elapsed_ms=~60000`
- `Hatchdoor starting host=0.0.0.0 port=42824 vault_path=/home/battermanz/notes cache_db_path=./tmp/hatchdoor-cache.sqlite3`
- `Hatchdoor listening on http://0.0.0.0:42824`

If `chunks_embedded` is `0` or `per_note_failures` is non-zero, stop and investigate before proceeding.

- [ ] **Step 3: Confirm existing endpoints still work**

In a second terminal:

```bash
curl -s http://127.0.0.1:42824/health
curl -s http://127.0.0.1:42824/api/tree | head -c 400
curl -s 'http://127.0.0.1:42824/api/recently-modified?limit=3' | jq .
```

Expected: `ok`; a non-empty tree JSON; recent notes from your real vault. Phase 1 is additive only — every existing endpoint must behave exactly as before.

- [ ] **Step 4: Inspect the cache to confirm chunks and vectors landed**

```bash
sqlite3 ./tmp/hatchdoor-cache.sqlite3 \
  "SELECT (SELECT COUNT(*) FROM notes), (SELECT COUNT(*) FROM chunks), (SELECT COUNT(*) FROM chunk_vectors), (SELECT value FROM metadata WHERE key='schema_version');"
```

Expected output: `286 | <N> | <N> | 3` — note count matches the vault, chunk and vector counts are equal and non-zero, schema version is `3`.

- [ ] **Step 5: Smoke-test the watcher path**

In a third terminal:

```bash
echo "# Watcher Probe\n\ntesting incremental indexing" > /home/battermanz/notes/_watcher_probe.md
```

Watch the server logs. Within a few seconds:
- An info line acknowledging the refresh.
- For the modified note only: `chunks_embedded >= 1`, `chunks_reused = 0`.

Then edit the same file (append a paragraph), save, and confirm the next log shows `chunks_embedded >= 1` and `chunks_reused = 1` (the unchanged sections were not re-embedded). Delete `_watcher_probe.md` when done.

- [ ] **Step 6: Smoke-test `semantic_search` via a one-shot test**

Add a temporary `#[ignore]` integration test in `tests/semantic_real.rs` (create the file):

```rust
#[test]
#[ignore]
fn semantic_search_against_real_cache() {
    use std::path::PathBuf;
    use std::sync::Arc;

    use hatchdoor::cache::SqliteCache; // make sure these are pub(crate); if not, add a test-only re-export
    use hatchdoor::embed::{ArcticEmbedder, Embedder};

    let cache = SqliteCache::open(PathBuf::from("./tmp/hatchdoor-cache.sqlite3")).expect("open");
    let embedder: Arc<dyn Embedder> = Arc::new(ArcticEmbedder::load().expect("load"));
    let hits = cache.semantic_search(embedder.as_ref(), "agents and tool use", 5).expect("search");
    assert!(!hits.is_empty(), "expected at least one hit against the real vault");
    for hit in &hits {
        println!("{:.4}  {}  ({})", hit.distance, hit.note_slug, hit.heading_path.as_deref().unwrap_or(""));
    }
}
```

If the crate's modules are private and adding `pub(crate)` re-exports is too invasive, skip this step — Task 12 already covered the SQL-shape regression via in-memory tests, and Phase 1.5 will exercise this against the real cache through its own harness.

Run: `cargo test --test semantic_real -- --ignored --nocapture`
Expected: prints five hits with reasonable note slugs from your vault.

- [ ] **Step 7: Commit (only if any source changed during verification)**

If you added the integration test in Step 6 and kept it:

```bash
git add tests/semantic_real.rs
git commit -m "$(cat <<'EOF'
test(cache): add ignored real-cache semantic_search smoke test

- One #[ignore] integration test that opens the on-disk cache and runs semantic_search through the real ArcticEmbedder, used as a manual sanity check before Docker packaging.
EOF
)"
```

Otherwise nothing to commit here.

---

## Task 17: Full quality gate

**Files:** none modified — verification only.

- [ ] **Step 1: Format + lint + tests**

Run: `cargo fmt && cargo check && cargo clippy --all-targets -- -D warnings && cargo test --all-targets`
Expected: clean. Fix any clippy warnings inline; if a fix changes behaviour, add a regression test first.

- [ ] **Step 2: Real-model test pass**

Run: `cargo test --features embedder-tests --all-targets`
Expected: clean. Weights already cached from Task 4; this should be quick.

- [ ] **Step 3: Commit any clippy/format adjustments**

If anything changed:

```bash
git add -u
git commit -m "$(cat <<'EOF'
chore: clippy and rustfmt cleanup after Phase 1 implementation

- Address clippy warnings surfaced by --all-targets -D warnings after the new modules landed.
EOF
)"
```

---

## Task 18: Docker — bake weights into the runtime image

**Files:**
- Modify: `Dockerfile`

This is the **last** task. Do not touch the Dockerfile before everything above is green. The image is the only artifact that needs the model weights baked in (local development uses fastembed's user cache directory).

- [ ] **Step 1: Add the prefetch step to the builder stage**

In `Dockerfile`, in the `rust-builder` stage, append after the existing `RUN cargo build --release --bin hatchdoor` line:

```dockerfile
# Pre-fetch embedder weights so the runtime image needs no network access.
ENV FASTEMBED_CACHE_PATH=/opt/fastembed
RUN mkdir -p $FASTEMBED_CACHE_PATH \
 && ./target/release/hatchdoor --prefetch-embedder
```

Do not modify the existing `COPY` or `RUN cargo build` lines.

- [ ] **Step 2: Surface the model dir into the runtime stage**

In the `runtime` stage `ENV` block (the one already containing `HOST`, `PORT`, `VAULT_PATH`, `RUST_LOG`), append exactly one line:

```dockerfile
    FASTEMBED_CACHE_PATH=/opt/fastembed \
```

(Place it before the trailing `RUST_LOG=...` line so the existing line continuations still align.)

Then, in the same stage, immediately after the existing `COPY --from=rust-builder /app/target/release/hatchdoor /app/hatchdoor` line, add:

```dockerfile
COPY --from=rust-builder /opt/fastembed /opt/fastembed
```

Do not re-emit the rest of the `ENV` block, the `COPY --from=frontend-builder` line, the `EXPOSE`, the `USER`, or the `ENTRYPOINT` lines. They stay exactly as they are.

- [ ] **Step 3: Build the image**

Run: `docker build -t hatchdoor:phase1 .`
Expected: a successful build. The build log shows the "Pre-fetching Arctic Embed S weights and tokenizer" line. Final image is ~500 MB. If the build fails because `nonroot` cannot read `/opt/fastembed`, append `&& chmod -R a+r /opt/fastembed` to the prefetch RUN line in the builder stage and rebuild.

- [ ] **Step 4: Verify the runtime image is self-contained**

Run: `docker run --rm hatchdoor:phase1 --prefetch-embedder`
Expected: the container starts, logs "Pre-fetch complete" almost immediately, and exits 0. If it tries to download from Hugging Face, `FASTEMBED_CACHE_PATH` is missing from the runtime env or the COPY line did not bring the cache over.

- [ ] **Step 5: Run against the real vault**

```bash
docker run --rm \
  -v /home/battermanz/notes:/data/vault:ro \
  -v $(pwd)/tmp:/data/cache \
  -e HATCHDOOR_CACHE_DB=/data/cache/hatchdoor-cache.sqlite3 \
  -p 42824:42824 \
  hatchdoor:phase1
```

Expected: the same log sequence as Task 16 step 2, and the same `curl` checks from Task 16 step 3 succeed against `http://127.0.0.1:42824/`.

- [ ] **Step 6: Commit**

```bash
git add Dockerfile
git commit -m "$(cat <<'EOF'
build(docker): bake Arctic Embed S weights into the runtime image

- Add a --prefetch-embedder RUN step to the rust-builder stage that downloads the model into /opt/fastembed during image build.
- Surface /opt/fastembed into the runtime stage and set FASTEMBED_CACHE_PATH so the running binary loads weights from the image without contacting Hugging Face.
- Accept the resulting ~200 MB image size growth in exchange for deterministic, offline-friendly cold starts.
EOF
)"
```

- [ ] **Step 7: Push the branch (optional)**

If the user wants the work on `origin/development`:

```bash
git push origin development
```

Otherwise leave it local.

---

## Out of scope (per spec §8)

These do NOT belong in this plan. Do not implement them even if they look easy from inside this work.

- New HTTP routes (semantic search via HTTP) — Phase 2.
- New MCP tools — Phase 2.
- Hybrid retrieval (FTS5 + semantic combined) — Phase 2.
- Phase 1.5 eval harness — its own plan, consumes `semantic_search`.
- Note-level hash skipping (skip the chunker if `notes.content_hash` is unchanged) — Phase 3.
- Multiple embedding backends, embedding versioning, model swaps — YAGNI.

---

## Self-review summary

- Every spec section maps to a task: §3 module layout → Tasks 3/4/8/9/10; §4 data model → Task 6; §5 indexing flow → Task 11; §6 failure modes → Task 14 (startup exit) + Task 15 (per-note WARN); §7 testing → covered per task; §10 alternatives → recorded in spec; §11 gotchas → Tasks 7, 11, 12, 15, 18 and the trait method in Task 3.
- Task 1 retires `VAULT_REFRESH_SECONDS`/`refresh_interval` so Phase 1 does not inherit dead plumbing — separate prep step, not bundled into the new functionality.
- Tasks 1–17 are entirely local; Task 18 is the only Docker work and lands last.
- The Dockerfile edit in Task 18 is now a minimal diff — one `ENV` line and one `COPY` line added, nothing existing re-emitted.
- No placeholders left in the plan. Every code-changing step has actual code or an explicit "match the project's existing pattern at file:line" callout.
- Type names are consistent: `Embedder`, `StubEmbedder`, `ArcticEmbedder`, `Chunk`, `ChunkOptions`, `NoteChunking`, `ChunkRow`, `ChunkStats`, `SemanticHit`, `UpsertOutcome` are defined where introduced and reused with the same shape downstream.
