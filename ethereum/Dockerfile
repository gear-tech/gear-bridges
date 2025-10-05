FROM ubuntu:22.04
WORKDIR /contracts
COPY . .
ENV PATH="${PATH}:/root/.foundry/bin"
ENV NVM_DIR="/root/.nvm"
RUN apt update && \
    apt install --no-install-recommends --yes \
        ca-certificates \
        curl \
        git \
        lcov && \
    curl -L https://foundry.paradigm.xyz | bash && \
    foundryup && \
    forge --version && \
    forge compiler resolve && \
    curl -o- https://raw.githubusercontent.com/nvm-sh/nvm/v0.40.3/install.sh | bash && \
    \. "$NVM_DIR/nvm.sh"  && \
    nvm install --lts && \
    npm install -g npm@latest && \
    node --version && \
    npm --version && \
    npx @openzeppelin/upgrades-core@^1.37.0 && \
    rm -rf /var/lib/apt/lists/*
