FROM debian:12-slim as builder

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
    wget \
    gcc \
    protobuf-compiler \
    clang

# Install rust
RUN curl https://sh.rustup.rs -sSf | bash -s -- -y
ENV PATH="/root/.cargo/bin:${PATH}"

# Install wasm-opt
RUN cargo install wasm-opt

# Install go
ENV GO_VERSION 1.20.1
RUN wget -P /tmp "https://go.dev/dl/go${GO_VERSION}.linux-amd64.tar.gz" && \
    tar -C /usr/local -xzf "/tmp/go${GO_VERSION}.linux-amd64.tar.gz" && \
    rm "/tmp/go${GO_VERSION}.linux-amd64.tar.gz"
ENV PATH /go/bin:/usr/local/go/bin:$PATH

# Install foundry
RUN curl -L https://foundry.paradigm.xyz | bash && \
    /root/.foundry/bin/foundryup
ENV PATH="/root/.foundry/bin:${PATH}"

COPY . .

# Build relayer
RUN cargo build -p relayer --release

# Compose final image
FROM debian:12-slim

RUN apt-get update && \
    apt-get install -y ca-certificates

COPY --from=builder /target/release/relayer .

EXPOSE 9090

ENTRYPOINT ["./relayer"]
