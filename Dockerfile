# syntax=docker.io/docker/dockerfile:1.7

FROM docker.io/library/rust:1.97-slim AS chef
WORKDIR /app
RUN apt-get update && apt-get install -y git pkg-config libssl-dev g++ perl make && rm -rf /var/lib/apt/lists/*
# Permission-sensitive tests must run without root; all Cargo stages share this user.
RUN useradd --uid 1000 --create-home builder \
    && chown builder:builder /app /usr/local/cargo /usr/local/cargo/bin
USER builder
ARG TARGETPLATFORM
ARG CARGO_CACHE_NAMESPACE=hatchdoor
# Unset by default: callers may bound Cargo's jobs without changing other builds.
ARG CARGO_BUILD_JOBS
ENV CARGO_TARGET_DIR=/app/target
RUN --mount=type=cache,id=${CARGO_CACHE_NAMESPACE}-registry-${TARGETPLATFORM},target=/usr/local/cargo/registry,sharing=locked,uid=1000,gid=1000 \
    --mount=type=cache,id=${CARGO_CACHE_NAMESPACE}-git-${TARGETPLATFORM},target=/usr/local/cargo/git,sharing=locked,uid=1000,gid=1000 \
    --mount=type=cache,id=${CARGO_CACHE_NAMESPACE}-target-${TARGETPLATFORM},target=/app/target,sharing=locked,uid=1000,gid=1000 \
    cargo install cargo-chef --version 0.1.77 --locked

FROM chef AS planner
COPY --chown=builder:builder Cargo.toml Cargo.lock ./
COPY --chown=builder:builder src ./src
COPY --chown=builder:builder docs/user-vault ./docs/user-vault
RUN cargo chef prepare --recipe-path recipe.json

FROM chef AS dependencies
# Preserve release defaults; incremental reuse within changed crates is opt-in.
ARG CARGO_PROFILE_RELEASE_INCREMENTAL=false
ARG CARGO_PROFILE_RELEASE_CODEGEN_UNITS=16
COPY --from=planner --chown=builder:builder /app/recipe.json recipe.json
RUN --mount=type=cache,id=${CARGO_CACHE_NAMESPACE}-registry-${TARGETPLATFORM},target=/usr/local/cargo/registry,sharing=locked,uid=1000,gid=1000 \
    --mount=type=cache,id=${CARGO_CACHE_NAMESPACE}-git-${TARGETPLATFORM},target=/usr/local/cargo/git,sharing=locked,uid=1000,gid=1000 \
    --mount=type=cache,id=${CARGO_CACHE_NAMESPACE}-target-${TARGETPLATFORM},target=/app/target,sharing=locked,uid=1000,gid=1000 \
    cargo chef cook --release --locked --recipe-path recipe.json

FROM dependencies AS rust-builder
COPY --chown=builder:builder Cargo.toml Cargo.lock ./
COPY --chown=builder:builder src ./src
COPY --chown=builder:builder docs/user-vault ./docs/user-vault
ARG GIT_SHA=""
# `docker` or `podman`, for the opt-in usage report; empty reports `source`.
ARG HATCHDOOR_IMAGE=""
# Cache mounts are not image layers: export the binary before unmounting target.
RUN --mount=type=cache,id=${CARGO_CACHE_NAMESPACE}-registry-${TARGETPLATFORM},target=/usr/local/cargo/registry,sharing=locked,uid=1000,gid=1000 \
    --mount=type=cache,id=${CARGO_CACHE_NAMESPACE}-git-${TARGETPLATFORM},target=/usr/local/cargo/git,sharing=locked,uid=1000,gid=1000 \
    --mount=type=cache,id=${CARGO_CACHE_NAMESPACE}-target-${TARGETPLATFORM},target=/app/target,sharing=locked,uid=1000,gid=1000 \
    HATCHDOOR_GIT_SHA=$GIT_SHA HATCHDOOR_IMAGE=$HATCHDOOR_IMAGE cargo build --locked --release --bin hatchdoor \
    && mkdir -p /app/artifacts \
    && cp /app/target/release/hatchdoor /app/artifacts/hatchdoor

FROM chef AS verification
COPY --chown=builder:builder Cargo.toml Cargo.lock ./
COPY --chown=builder:builder src ./src
COPY --chown=builder:builder docs/user-vault ./docs/user-vault
# The SPA regression test compares reserved routes with the real worker denylist.
COPY --chown=builder:builder frontend/vite.config.ts ./frontend/vite.config.ts
ARG GIT_SHA=""
RUN --mount=type=cache,id=${CARGO_CACHE_NAMESPACE}-registry-${TARGETPLATFORM},target=/usr/local/cargo/registry,sharing=locked,uid=1000,gid=1000 \
    --mount=type=cache,id=${CARGO_CACHE_NAMESPACE}-git-${TARGETPLATFORM},target=/usr/local/cargo/git,sharing=locked,uid=1000,gid=1000 \
    --mount=type=cache,id=${CARGO_CACHE_NAMESPACE}-target-${TARGETPLATFORM},target=/app/target,sharing=locked,uid=1000,gid=1000 \
    HATCHDOOR_GIT_SHA=$GIT_SHA cargo test --locked

FROM docker.io/library/node:26-slim AS frontend-builder
WORKDIR /app/frontend
COPY frontend/package.json frontend/package-lock.json ./
RUN npm ci
COPY frontend ./
RUN npm run build

FROM gcr.io/distroless/cc-debian13:nonroot@sha256:d97bc0a941b8d4be647dc0ee75b264ddbb772f1ac5ba690a4309c00723b23775 AS runtime
WORKDIR /app

ENV HOST=0.0.0.0 \
    PORT=42824 \
    VAULT_PATH=/data/vault \
    RUST_LOG=hatchdoor=info,tower_http=info,axum::rejection=warn

COPY --from=rust-builder /app/artifacts/hatchdoor /app/hatchdoor
COPY --from=frontend-builder /app/frontend/dist /app/frontend/dist

EXPOSE 42824
USER nonroot:nonroot
ENTRYPOINT ["/app/hatchdoor"]

# Last, so a new commit or version rewrites only metadata and no layer above.
# description, licenses and source repeat Cargo.toml, and
# scripts/dockerfile-labels.test.mjs fails when they drift. The MCP registry
# reads the server name to prove who owns the image.
ARG VERSION=""
ARG GIT_SHA=""
LABEL org.opencontainers.image.title="Hatchdoor" \
      org.opencontainers.image.description="Self-hosted Obsidian-style Markdown vault browser with optional web and MCP write access" \
      org.opencontainers.image.licenses="AGPL-3.0-only" \
      org.opencontainers.image.source="https://github.com/BatterWorks/Hatchdoor" \
      org.opencontainers.image.url="https://github.com/BatterWorks/Hatchdoor" \
      org.opencontainers.image.documentation="https://docs-hatchdoor.battercloud.cc" \
      org.opencontainers.image.version="${VERSION}" \
      org.opencontainers.image.revision="${GIT_SHA}" \
      io.modelcontextprotocol.server.name="io.github.BatterWorks/hatchdoor"
