# syntax=docker.io/docker/dockerfile:1.7

FROM docker.io/library/rust:1.97-slim AS chef
WORKDIR /app
RUN apt-get update && apt-get install -y pkg-config libssl-dev g++ perl make && rm -rf /var/lib/apt/lists/*
RUN cargo install cargo-chef --version 0.1.77 --locked

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

COPY --from=rust-builder /app/target/release/hatchdoor /app/hatchdoor
COPY --from=frontend-builder /app/frontend/dist /app/frontend/dist

EXPOSE 42824
USER nonroot:nonroot
ENTRYPOINT ["/app/hatchdoor"]
