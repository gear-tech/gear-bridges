FROM debian:12-slim AS native-builder
SHELL ["/bin/bash", "-o", "pipefail", "-c"]
RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates curl build-essential pkg-config libssl-dev cmake git protobuf-compiler clang \
    && rm -rf /var/lib/apt/lists/*
RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --default-toolchain none
ENV PATH="/root/.cargo/bin:${PATH}"
WORKDIR /app
COPY rust-toolchain.toml .
RUN rustup show
COPY . .
RUN --mount=type=cache,target=/root/.cargo/registry \
    --mount=type=cache,target=/root/.cargo/git \
    --mount=type=cache,target=/app/target \
    cargo build --locked --release -p relayer --bin relayer \
    && install -D target/release/relayer /native/relayer

# The native verifier and Node share Debian 12's glibc/OpenSSL runtime.
FROM node:20-bookworm-slim AS node
RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates libssl3 g++ make python3 && rm -rf /var/lib/apt/lists/*
RUN corepack enable

FROM node AS builder
COPY package.json yarn.lock lerna.json .yarnrc.yml /app/
COPY .yarn /app/.yarn
COPY api /app/api
COPY js/indexer /app/js/indexer
COPY js/common /app/js/common
WORKDIR /app
RUN yarn workspaces focus indexer gear-bridges gear-bridge-common
RUN yarn build:indexer

FROM node AS deps
WORKDIR /app
COPY package.json yarn.lock lerna.json .yarnrc.yml /app/
COPY .yarn /app/.yarn
COPY js/indexer /app/js/indexer
COPY js/common /app/js/common
RUN yarn workspaces focus gear-bridges indexer gear-bridge-common --production

FROM deps
COPY --from=native-builder /native/relayer /usr/local/bin/relayer
ENV ETH_EVIDENCE_BINARY=/usr/local/bin/relayer
COPY --from=builder /app/js/indexer/lib /app/js/indexer/lib
COPY --from=builder /app/js/indexer/db /app/js/indexer/db
COPY --from=builder /app/js/common/lib /app/js/common/lib
COPY --from=builder /app/api /app/api
WORKDIR /app
