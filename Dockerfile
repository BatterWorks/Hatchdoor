# syntax=docker/dockerfile:1.7

FROM rust:1.96-slim AS chef
WORKDIR /app
RUN apt-get update && apt-get install -y pkg-config libssl-dev g++ perl make && rm -rf /var/lib/apt/lists/*
RUN cargo install cargo-chef --locked

FROM chef AS planner
COPY Cargo.toml Cargo.lock ./
COPY src ./src
COPY docs/starter-vault ./docs/starter-vault
RUN cargo chef prepare --recipe-path recipe.json

FROM chef AS rust-builder
COPY --from=planner /app/recipe.json recipe.json
RUN cargo chef cook --release --recipe-path recipe.json
COPY Cargo.toml Cargo.lock ./
COPY src ./src
COPY docs/starter-vault ./docs/starter-vault
RUN cargo build --release --bin hatchdoor
ENV FASTEMBED_CACHE_DIR=/opt/fastembed
RUN mkdir -p $FASTEMBED_CACHE_DIR \
 && ./target/release/hatchdoor --prefetch-embedder

FROM node:24-slim AS frontend-builder
WORKDIR /app/frontend
COPY frontend/package.json frontend/package-lock.json ./
RUN npm ci
COPY frontend ./
RUN npm run build

FROM gcr.io/distroless/cc-debian13:nonroot AS runtime
WORKDIR /app

ENV HOST=0.0.0.0 \
    PORT=42824 \
    VAULT_PATH=/data/vault \
    FASTEMBED_CACHE_DIR=/opt/fastembed \
    RUST_LOG=hatchdoor=info,tower_http=info,axum::rejection=warn

COPY --from=rust-builder /app/target/release/hatchdoor /app/hatchdoor
COPY --from=rust-builder /opt/fastembed /opt/fastembed
COPY --from=frontend-builder /app/frontend/dist /app/frontend/dist

EXPOSE 42824
USER nonroot:nonroot
ENTRYPOINT ["/app/hatchdoor"]
