# Dependency update plan

> Maintenance record.

Status: all seven steps complete. FastEmbed 5 is deferred because it would
impose an AVX requirement on x86_64 users.

Inventory checked on 2026-07-21. Direct dependencies are listed below; their
transitive dependencies will be refreshed through `Cargo.lock` and
`frontend/package-lock.json`, rather than manually pinned one by one.

## Target inventory

### Rust

| Package | Current resolved | Target |
| --- | ---: | ---: |
| ahash | 0.8.12 | retain |
| axum | 0.8.9 | retain |
| base64 | 0.22.1 | retain |
| blake3 | 1.8.5 | retain |
| bytemuck | 1.25.2 | retain |
| chrono | 0.4.45 | retain |
| dotenvy | 0.15.7 | retain |
| fastembed | 4.9.1 | retain (v5 deferred: requires AVX here) |
| git2 | 0.21.0 | retain |
| libc | 0.2.186 | 0.2.188 |
| notify | 8.2.0 | retain |
| rusqlite | 0.39.0 | 0.40.1 |
| serde | 1.0.229 | retain |
| serde_json | 1.0.150 | 1.0.151 |
| serde_yaml | 0.9.34+deprecated | retain — superseded by #196, replaced with serde_yaml_ng 0.10 |
| sqlite-vec | 0.1.9 | retain |
| text-splitter | 0.27.0 | 0.32.0 |
| tokenizers | 0.21.4 | 0.23.1 |
| tokio | 1.53.0 | 1.53.1 |
| tokio-stream | 0.1.18 | retain |
| tower-http | 0.6.11 | 0.7.0 |
| tracing | 0.1.44 | retain |
| tracing-subscriber | 0.3.23 | retain |
| walkdir | 2.5.0 | retain |
| zip | 2.4.2 | 8.6.0 |
| dev: tempfile | 3.27.0 | retain |
| dev: tower | 0.5.3 | retain |

### Frontend

| Package | Current declared | Target |
| --- | ---: | ---: |
| d3-force | ^3.0.0 | retain |
| mermaid | ^11.16.0 | retain |
| react, react-dom | ^19.2.7 | ^19.2.8 |
| react-markdown | ^10.1.0 | retain |
| react-router-dom | ^7.18.1 | retain |
| rehype-katex | ^7.0.1 | retain |
| remark-gfm | ^4.0.1 | retain |
| remark-math | ^6.0.0 | retain |
| vite-plugin-pwa | ^1.3.0 | retain |
| @babel/core | — | ^7.29.7 |
| @eslint/js | ^9.39.5 | ^10.0.1 |
| @resvg/resvg-js | ^2.6.2 | retain |
| @rolldown/plugin-babel | — | 0.1.8 |
| @testing-library/jest-dom | ^6.9.1 | ^7.0.0 |
| @testing-library/react | ^16.3.2 | retain |
| @types/node | ^24.13.3 | ^26.1.1 |
| @types/react | ^19.2.17 | retain |
| @types/react-dom | ^19.2.3 | retain |
| @vitejs/plugin-react | ^5.2.0 | ^6.0.3 |
| @vitest/coverage-v8 | ^4.1.10 | retain |
| eslint | ^9.39.5 | ^10.7.0 |
| eslint-plugin-react-hooks | ^7.0.1 | ^7.1.1 |
| eslint-plugin-react-refresh | ^0.4.24 | ^0.5.3 |
| globals | ^16.5.0 | ^17.7.0 |
| jsdom | ^28.1.0 | ^29.1.1 |
| png-to-ico | ^3.0.2 | retain |
| prettier | ^3.9.5 | ^3.9.6 |
| typescript | ~5.9.3 | 6.0.3 (see exception) |
| typescript-eslint | ^8.64.0 | ^8.65.0 |
| vite | ^7.3.6 | ^8.1.5 |
| vitest | ^4.1.10 | retain |

`typescript@7` is intentionally deferred: the current `typescript-eslint`
peer range ends before TypeScript 6.1, and TypeScript 7 does not yet provide
the compatible programmatic API required by that toolchain.

### Build images and tools

- `rust-toolchain.toml`: Rust 1.96.0 -> 1.97.1.
- Rust builder: `rust:1.96-slim` -> `rust:1.97-slim`.
- Frontend builder: `node:24-slim` -> `node:26-slim`.
- Pin `cargo-chef` to 0.1.77 instead of installing an unpinned latest release.
- Keep the distroless Debian 13 runtime, but refresh and pin its image digest.

## Implementation sequence

1. **Complete.** Update low-risk lockfile changes first: Cargo-compatible updates, React
   patches, Prettier, and other non-major frontend packages. Run the full Rust
   and frontend checks. Do not build Docker images.
2. **Complete.** Upgrade the frontend toolchain together: Node 26, Vite 8, plugin-react 6,
   ESLint 10, jsdom 29, and related types/lint packages. Vite 8 changes its
   bundler internals to Rolldown, so verify the custom `manualChunks` setup,
   PWA generation, development server, production build, and browser smoke
   tests.
3. **Implemented.** Upgrade `text-splitter` and `tokenizers` as one
   semantic-index change. Per request, skip the before-upgrade baseline.
   FastEmbed v5 is deferred: its ONNX Runtime 1.24 static binary introduces an
   AVX requirement on x86_64, which Hatchdoor must not impose. Token counting
   is performed through the embedder, so chunking continues to use the model's
   actual tokenizer despite the direct `tokenizers` 0.23 upgrade. Schema version
   7 and the `fastembed-v4` cache identity force one clean full reindex,
   preventing vectors from different runtimes being mixed. The full Rust suite,
   strict Clippy, and a clean BGE-small reindex plus semantic retrieval check on
   an isolated vault passed.
4. **Complete.** Upgrade `rusqlite` to 0.40.1, `tower-http` to 0.7.0, and
   `zip` to 8.6.0. The full Rust suite covers SQLite extension loading,
   FTS/vector queries, static assets, and generated ZIP downloads; all tests and
   strict Clippy passed. `rusqlite`'s virtual-table API changes do not affect
   Hatchdoor directly.
5. **Complete.** Regenerated both lockfiles and ran formatting, Rust tests,
   strict Clippy, `cargo audit`, frontend typecheck/lint/tests/build, and
   `npm audit` without building Docker images. Rust has 301 passing tests and
   no audit vulnerabilities; its audit reports two unmaintained transitive
   crates (`number_prefix` and `paste`) inherited through FastEmbed 4 and
   tokenizers. Frontend audit reports zero vulnerabilities. Prettier reports
   four pre-existing files outside this dependency update; they were left
   unchanged rather than mixing unrelated formatting into the upgrade.
6. **Complete.** Exercised the built app in an isolated temporary vault and
   cache. A fresh Nomic/FastEmbed 4 index embedded two chunks successfully;
   restart against that cache reported zero updated notes and zero newly
   embedded chunks. HTTP semantic search, SPA/PWA manifest and service worker,
   valid PNG upload, vault asset serving, and note ZIP export (including its
   referenced asset) passed. The real Jina Turbo reranker also passed an
   isolated retrieval query (Recall@5=1.0, MRR=1.0). The temporary vault,
   cache, exports, and build output were removed after validation.
7. **Complete.** Updated Rust to 1.97.1 (including the Docker builder), pinned
   `cargo-chef` to 0.1.77, and pinned the Debian 13 distroless non-root runtime
   to `sha256:d97bc0a941b8d4be647dc0ee75b264ddbb772f1ac5ba690a4309c00723b23775`.
   The application was checked with Rust 1.97.1 in an isolated Cargo target.
   No Docker image was built.

## Upstream references

- [Vite 8 migration guide](https://vite.dev/guide/migration.html)
- [ESLint 10 migration guide](https://eslint.org/docs/latest/use/migrate-to-10.0.0)
- [TypeScript 7 announcement](https://devblogs.microsoft.com/typescript/announcing-typescript-7-0/)
- [fastembed v5 release](https://github.com/Anush008/fastembed-rs/releases/tag/v5.0.0)
- [text-splitter documentation](https://docs.rs/crate/text-splitter/latest)
- [rusqlite releases](https://github.com/rusqlite/rusqlite/releases)
- [zip changelog](https://github.com/zip-rs/zip2/blob/main/CHANGELOG.md)
