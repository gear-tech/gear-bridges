FROM debian:12-slim AS builder

SHELL ["/bin/bash", "-c"]

# Install deps
RUN apt-get update && \
    apt-get install -y \
    curl \
    build-essential \
    pkg-config \
    libssl-dev \
    cmake \
    git \
    gcc \
    protobuf-compiler \
    clang

# Install rust
RUN curl https://sh.rustup.rs -sSf | bash -s -- -y
ENV PATH="/root/.cargo/bin:${PATH}"

# Install wasm-opt
RUN cargo install wasm-opt

# Install foundry
RUN curl -L https://foundry.paradigm.xyz | bash && \
    /root/.foundry/bin/foundryup --install v1.8.5
ENV PATH="/root/.foundry/bin:${PATH}"

COPY . .

# Build delivery, BEEFY publication and explicit deployment executables
RUN rustup toolchain install
RUN cargo build --locked --release -p relayer -p beefy-relay --features beefy-relay/deployment

# Compose final image
FROM debian:12-slim

RUN apt-get update && \
    apt-get install -y ca-certificates

COPY --from=builder /target/release/relayer /target/release/beefy-relay /target/release/beefy-deploy ./

EXPOSE 9090

ENTRYPOINT ["./relayer"]
