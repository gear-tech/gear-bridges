FROM ubuntu:22.04 as builder
SHELL ["/bin/bash", "-c"]

# Install deps
RUN apt-get update
RUN apt-get install -y \
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
RUN wget -P /tmp "https://go.dev/dl/go1.20.1.linux-amd64.tar.gz"
RUN tar -C /usr/local -xzf "/tmp/go1.20.1.linux-amd64.tar.gz"
RUN rm "/tmp/go1.20.1.linux-amd64.tar.gz"
ENV GOPATH /go
ENV PATH $GOPATH/bin:/usr/local/go/bin:$PATH
RUN mkdir -p "$GOPATH/src" "$GOPATH/bin" && chmod -R 777 "$GOPATH"

# Install foundry
RUN curl -L https://foundry.paradigm.xyz | bash
RUN /root/.foundry/bin/foundryup
ENV PATH="/root/.foundry/bin:${PATH}"

COPY . .

# Build relayer
RUN cargo build -p relayer --release

# Compose final image
FROM ubuntu:22.04
COPY --from=builder /target/release/relayer /usr/local/bin/relayer
CMD ["relayer"]