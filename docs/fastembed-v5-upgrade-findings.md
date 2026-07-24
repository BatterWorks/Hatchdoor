# FastEmbed v5 Upgrade — Feasibility Findings

> Investigation notes captured on 2026-07-24 on branch `feature/fastembed-v5-models`
> (off `development`). This is the *platform/build feasibility* base for a later
> v5 implementation. For the model-selection strategy, see the companion doc
> [`embeddings-investigation-fastembed-v5.md`](./embeddings-investigation-fastembed-v5.md).

## Background: the v4 → v5 → v4 excursion

During the dependency batch on `development` (commits `c7936e7`, `af80a79`,
2026-07-21), FastEmbed 5 was attempted and reverted. The revert reason recorded
in `docs/dependency-update-plan.md` was ONNX Runtime's AVX baseline requirement.
FastEmbed 5 was never committed — `git log -S 'fastembed = "5"'` on `Cargo.toml`
is empty. The excursion left two residues:

- `docs/dependency-update-plan.md` notes "FastEmbed 5 is deferred … requires AVX here".
- `Cargo.toml` keeps a `tokenizers-v21` alias, because FastEmbed 4 pins tokenizers 0.21
  while the rest of the tree moved to tokenizers 0.23.

This document re-examines whether that deferral still stands. **Conclusion: the
original blockers are resolved by the current distroless Debian 13 target.**

## Key finding 1 — AVX is an x86-only concern, irrelevant to ARM64

AVX / AVX2 / AVX-512 are x86-64 instruction sets. ARM64 (aarch64) CPUs — the
public demo host (`batteroraclearm`), Apple Silicon, Graviton — physically cannot
execute them and never will; their SIMD equivalent is NEON (and SVE). ONNX
Runtime's *x86-64* prebuilt binaries raised their baseline to require AVX, which
would fault on an old AVX-less x86 host. On aarch64 a separate NEON-compiled
binary is used and the AVX flag is a non-event. ARM64 is **not** "forever
incompatible" — the AVX issue does not apply to it at all.

## Key finding 2 — FastEmbed v5's backend ships prebuilt ARM64-Linux binaries

FastEmbed v5.0.0's breaking change was upgrading its ONNX Runtime binding to
`ort` `2.0.0-rc.10` (from ort 1.x) and removing Rayon. The `ort` crate's own
platform matrix (`pykeio/ort`, `docs/core/platform.tsx`, `PLATFORMS_WITH_BINARIES`)
explicitly includes prebuilt binaries for:

- `linux/arm64`  ← the demo host
- `linux/x64`    ← the primary deploy host
- `macos/arm64`, `windows/arm64`, `ios/arm64`, `android/arm64`

So an aarch64 container gets a native prebuilt binary with no build-from-source.

## Key finding 3 — the real runtime floor is glibc / libstdc++, and we clear it

`ort`'s prebuilt Linux binaries (x64 **and** arm64) require:

> **glibc ≥ 2.39 & libstdc++ ≥ 13.2** (Ubuntu ≥ 24.04, Debian ≥ 13 "Trixie").

This is a toolchain floor, not an ISA problem, and it applies on every arch.
Hatchdoor's runtime image is `gcr.io/distroless/cc-debian13:nonroot`
(`Dockerfile` line 39):

- **Debian 13 "Trixie"** → glibc ~2.41 (≥ 2.39 ✓)
- **`cc` variant** → ships `libgcc` + `libstdc++6` (from GCC 14, ≥ 13.2 ✓).
  Note: the `base-debian13` variant does **not** ship libstdc++; the `cc`
  variant is required and is what we use.

**The base image is not a blocker.**

## Remaining costs / decisions for the actual upgrade

These are not blockers, but they are not free either — budget for them.

1. **ort 2.0 API compile check.** Our FastEmbed surface is small and stable —
   `TextEmbedding::try_new(InitOptions::new(model).with_max_length(..).with_show_download_progress(false))`
   plus the `EmbeddingModel` enum (`src/embed/fastembed_embedder.rs:1,23-25`).
   These survived into v5, and the v5 breaking changes (ort 2.0, Rayon removal)
   don't obviously touch this surface — but confirm with an actual `cargo build`,
   don't assume.

2. **Forced full reindex (by design).** The cache keys on `embedder.identity()`,
   which hardcodes `...-fastembed-v4` (`src/embed/fastembed_embedder.rs:99`).
   Bumping to v5 means changing that string, which trips the identity-change
   rebuild path (`src/cache/schema.rs:56-63`) — **every deployment wipes its
   SQLite cache and re-embeds the whole vault on first boot after the upgrade**
   (demo + all clients). Handled gracefully, but ship it knowingly, not silently.
   (Cache schema is currently version `8`, `src/cache/schema.rs:9`.)

3. **Build-time AVX on the x86 build host.** The Dockerfile runs the embedder at
   *build* time (`--prefetch-embedder`, `Dockerfile:26-28`) to warm the model
   cache. On the x86 build leg this executes ONNX Runtime → needs AVX on whatever
   runs turbobuild. Modern build hosts have it; confirm before trusting CI. The
   arm64 build leg is unaffected.

4. **Drop the `tokenizers-v21` alias (cleanup).** The alias exists only because
   FastEmbed 4 pins tokenizers 0.21; its sole user is
   `src/bin/index_microbench.rs:7` (`use tokenizers_v21::Tokenizer;`). If v5 moves
   to tokenizers 0.23+, drop the alias and point the microbench at the main
   `tokenizers` dep. Optional, removes the wart the v4→v5→v4 saga left behind.

## Bottom line

The v5 deferral was correct for the *previous* target but is **no longer
justified** on distroless Debian 13. The upgrade is viable. It is not a pure
version bump: plan for an ort-2.0 compile pass, an identity change that forces a
one-time full reindex everywhere, and a build-host AVX sanity check.

## Sources

- pykeio/ort platform matrix — `docs/core/platform.tsx` (`PLATFORMS_WITH_BINARIES`)
- FastEmbed v5.0.0 release (ort 2.0.0-rc.10 upgrade, Rayon removal)
- `docs/dependency-update-plan.md` (original v5 deferral rationale)
- Local: `Dockerfile:39`, `src/embed/fastembed_embedder.rs:99`, `src/cache/schema.rs:9,56-63`
