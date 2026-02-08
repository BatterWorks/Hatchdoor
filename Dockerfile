# syntax=docker/dockerfile:1.7

FROM rust:1.93-bookworm AS chef
WORKDIR /app
RUN cargo install cargo-chef --locked

FROM chef AS planner
COPY Cargo.toml Cargo.lock ./
COPY src ./src
RUN cargo chef prepare --recipe-path recipe.json

FROM chef AS rust-builder
COPY --from=planner /app/recipe.json recipe.json
RUN cargo chef cook --release --recipe-path recipe.json
COPY Cargo.toml Cargo.lock ./
COPY src ./src
RUN cargo build --release --bin hatchdoor

FROM node:22-bookworm-slim AS frontend-builder
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
    VAULT_REFRESH_SECONDS=2 \
    RUST_LOG=hatchdoor=info,tower_http=info,axum::rejection=warn

COPY --from=rust-builder /app/target/release/hatchdoor /app/hatchdoor
COPY --from=frontend-builder /app/frontend/dist /app/frontend/dist

EXPOSE 42824
USER nonroot:nonroot
ENTRYPOINT ["/app/hatchdoor"]
