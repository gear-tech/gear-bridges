# Running the bridge

This page is an operator guide for the `relayer` binary. It covers the core protocol relayers and the token-relayer processes that consume the protocol's verified messages.

## What you are starting

The binary has separate subcommands for each direction and layer:

| Command | Role |
| --- | --- |
| `gear-eth-core` | Reads finalized Gear blocks, proves message-queue roots, and submits the proofs to Ethereum. |
| `eth-gear-core` | Reads Ethereum beacon-chain finality data and updates the Gear checkpoint-light-client program. |
| `gear-eth-tokens` | Consumes Gear-side token messages and submits the corresponding Ethereum transactions. |
| `eth-gear-tokens` | Consumes Ethereum token events and submits the corresponding Gear messages. |
| `gear-eth-manual` / `eth-gear-manual` | Replays one known message when an automated token relayer needs operator assistance. |
| `kill-switch` | Watches for emergency-stop events and calls a configured relayer HTTP endpoint. |
| `queue-cleaner` | Performs the Gear queue-cleaner maintenance operation. |
| `fetch-merkle-roots` | Fetches roots already relayed to Ethereum for inspection/recovery workflows. |
| `update-verifier-sol` | Runs the proof-generation utility used when regenerating verifier material. |

The root [README](../README.md) explains the protocol-level message and token flows. The [internals](internals.md) page maps these commands to their implementation components.

## Prerequisites

For a native build, install the toolchains used by the workspace:

- Rust, using the repository's [rust-toolchain.toml](../rust-toolchain.toml).
- Go, for [gnark-wrapper](../gnark-wrapper).
- Foundry (forge/cast) for the Ethereum contracts and deployment tooling.
- The native build dependencies listed in [Dockerfile](../Dockerfile), including a C compiler, OpenSSL development files, CMake, protobuf compiler, and Clang.

The root README also calls out the [ring build instructions](https://github.com/gear-tech/ring/blob/main/BUILDING.md). Follow those instructions when a native build fails while compiling `ring`.

Build the relayer from the repository root:

~~~sh
cargo build --release -p relayer
target/release/relayer --help
~~~

Proof generation uses large native stacks. `gear-eth-core` requires `RUST_MIN_STACK` to be set to at least 4 MiB before startup. A typical native invocation starts with:

~~~sh
export RUST_MIN_STACK=4194304
export RUST_LOG='relayer=info,prover=info,ethereum-client=info,metrics=info'
~~~

Increase `RUST_MIN_STACK` or reduce proof worker counts when a host is memory constrained. Each configured proof thread can allocate substantial memory.

## Configure `gear-eth-core`

The current `relayer` CLI is flag- and environment-driven. There is no checked-in TOML configuration schema in the master-based branch. Use the command's help output as the authoritative list of required values:

~~~sh
target/release/relayer gear-eth-core --help
~~~

The core command combines connection, signer, genesis, Prometheus, proof-storage, and block-storage arguments. Important values include the Gear endpoint, Ethereum RPC endpoint, MessageQueue address, Ethereum fee-payer key, genesis authority-set hash and id, web-server token, and block-storage path.

Keep secrets in the process environment or an external secret manager. Keep block storage and proof storage on persistent volumes, and use a separate directory for each relayer process.

## Flag mode

Use `target/release/relayer gear-eth-core --help` for the exact current surface. A redacted shape is:

~~~sh
RUST_MIN_STACK=4194304 \
  target/release/relayer gear-eth-core \
  --gear-endpoint wss://gear.example \
  --ethereum-endpoint https://ethereum.example \
  --mq-address 0x...20-byte-address... \
  --eth-fee-payer 0x...32-byte-private-key... \
  --authority-set-hash 0x...32-byte-digest... \
  --authority-set-id 123 \
  --web-server-token "$RELAYER_HTTP_TOKEN" \
  --block-storage /var/lib/gear-bridges/merkle-roots.json
~~~

The flag names map to environment variables such as `GEAR_ENDPOINT`, `ETH_MESSAGE_QUEUE_ADDRESS`, `ETH_FEE_PAYER`, `GENESIS_CONFIG_AUTHORITY_SET_HASH`, `GENESIS_CONFIG_AUTHORITY_SET_ID`, `WEB_SERVER_TOKEN`, and `GEAR_BLOCK_STORAGE`. The CLI help is authoritative for defaults and required values. Commands for token relayers, manual relays, the kill switch, queue cleaner, root fetching, and verifier generation expose different argument groups; do not reuse a core command's flags without checking that subcommand's help.

## Run with Docker

The [Dockerfile](../Dockerfile) builds the complete relayer image. Build it from the repository root:

~~~sh
docker build -t gear-bridges-relayer:local -f Dockerfile .
~~~

When wrapping the image in Compose or another supervisor, mount the executable's configuration inputs, persistent block/proof storage, and verifier data explicitly. Set `RUST_MIN_STACK`, expose Prometheus and the authenticated HTTP port separately, and use a restart policy appropriate to the deployment. Render and inspect the final service definition before starting it.

~~~sh
docker compose config
docker compose up -d
docker compose ps
docker compose logs -f <service>
~~~

Do not copy network names, addresses, keys, or host paths from a local service definition into another environment.

## Ports and endpoints

There are two distinct HTTP surfaces:

- Prometheus metrics, configured with `--prometheus-endpoint`; the default is `0.0.0.0:9090`.
- The authenticated relayer HTTP server, configured with `--web-server-address`; its default is `127.0.0.1:8443`.

The server requires the `X-Token` header. Core relayers expose `POST /get_merkle_root_proof`; token relayers additionally expose `/relay_messages` or `/relay_transactions` depending on the direction and mode. See [usage and operations](usage-and-operations.md) for request bodies and response handling.

Bind the authenticated API to a private interface or protect it with network policy. Prometheus is unauthenticated at the application layer, so restrict its exposure to the monitoring network.

## Shutdown, restart, and recovery

Use the process supervisor or Docker to stop the service cleanly:

~~~sh
docker compose stop merkle-root-relayer
docker compose start merkle-root-relayer
~~~

The Merkle-root relayer saves its block/root/submission state periodically and on processing-loop iterations. Writes use a temporary file and a `.bak` copy before the primary file is replaced. On startup, the relayer restores pending proof-generation and submission work from that state and reconciles roots with Ethereum before retransmitting.

Keep the state file, its `.bak` file, proof-storage directory, verifier/SRS data, and transaction-storage directories together. Back them up before changing authority-set configuration or deleting old state. A genesis authority-set change changes the circuit's fixed genesis inputs and makes later proofs incompatible; coordinate such a change with the deployed verifier and proof storage.

For a transient RPC failure, the Gear listener reconnects and replays missing finalized blocks; Ethereum pollers reconnect and resume from their persisted cursors. A child task that exits permanently is surfaced as a process failure. Inspect logs and metrics before deciding whether to retry, restore state, or escalate to the emergency procedures.

## First-start checklist

Before allowing the service to submit transactions:

1. Verify the Gear endpoint is a dedicated, trusted node and can serve finalized blocks and GRANDPA justifications.
2. Verify the Ethereum RPC can read finalized state and the fee payer has enough native currency.
3. Verify the MessageQueue address, genesis authority-set hash/id, and verifier/SRS data belong to the same deployment.
4. Verify every persistent directory exists and is writable by the container user, while secret files are not world-readable.
5. Render the supervisor configuration and inspect every path and port.
6. Start with `RUST_LOG=relayer=info,prover=info` and confirm the relayer loads storage, initializes authority-set sync, starts its HTTP server, and exposes `/metrics`.
7. Only then enable the token relayers that consume roots and submit user-facing transfers.
