# Usage and operations

This guide is for operators and developers who already know the bridge topology and need to exercise it safely. Read [Running the bridge](running-the-bridge.md) for installation and configuration, [Internals](internals.md) for process flow, and [Merkle roots](merkle-roots.md) for root lifecycle details.

## Bridge directions

The repository contains two main message paths:

- Gear to Ethereum: finalized Gear state is converted into an Ethereum-verifiable Merkle-root proof. The relayer submits the root proof to the Ethereum MessageQueue contract.
- Ethereum to Gear: Ethereum transactions are observed through the configured RPC endpoint and delivered to Gear after the configured finality policy is satisfied.

Token relayers are separate processes. They use the same source and destination concepts, but their contracts, checkpoints, and failure handling are not interchangeable with the core message relayer.

## Choose the process

Use the command that matches the responsibility of the process:

| Responsibility | Command |
| --- | --- |
| Gear message queues to Ethereum | gear-eth-core |
| Ethereum message queues to Gear | eth-gear-core |
| Gear token events to Ethereum | gear-eth-tokens |
| Ethereum token events to Gear | eth-gear-tokens |
| Manual message forwarding | gear-eth-manual or eth-gear-manual |
| Stop or pause relaying | kill-switch |
| Remove expired queue entries | queue-cleaner |
| Fetch a root proof from a relayer | fetch-merkle-roots |
| Update the Ethereum verifier | update-solidity-verifier |

The current CLI is authoritative for command names and flags:

~~~text
cargo run --release --bin relayer -- --help
cargo run --release --bin relayer -- gear-eth-core --help
cargo run --release --bin relayer -- eth-gear-core --help
~~~

## Configuration workflow

The current relayer uses command-line flags and environment variables rather than a checked-in TOML schema. Start by printing help for the exact subcommand. Keep secrets outside the repository whenever the deployment system supports secret injection.

A practical workflow is:

1. Print the selected subcommand's help.
2. Set the Gear and Ethereum endpoints.
3. Set the correct network genesis values and destination addresses.
4. Set signing keys through the deployment secret mechanism.
5. Set storage and proof-storage paths on persistent volumes.
6. Configure the HTTP token if the management API is enabled.
7. Start one process with a low log level and verify health before enabling dependent token relayers.

The core settings fall into four groups:

- Chain identity: Gear RPC, Ethereum RPC, network genesis, and destination contracts.
- Cryptographic material: signer keys, verifier inputs, authority-set data, and gnark-wrapper paths.
- Durable state: relay state, proof state, checkpoints, and queues.
- Operations: HTTP bind address, authentication token, Prometheus address, polling periods, batching, thresholds, and retry policy.

## Starting locally

For a single development process:

~~~sh
RUST_LOG=info \
cargo run --release --bin relayer -- gear-eth-core --help
~~~

Use the equivalent command for the reverse direction. Supply the endpoint, key, genesis, contract, and storage arguments shown by that command's help.

The first startup should be supervised interactively. Confirm:

- The process selected the intended network and command.
- The configured endpoints are reachable.
- The signer account is the expected account.
- The relayer can read finalized blocks.
- Persistent state was created at the expected path.
- No proof generation or transaction submission is using an unintended temporary directory.
- The HTTP and Prometheus listeners bound to the intended interfaces.

## Running with Docker Compose

The repository includes a Dockerfile; a deployment may wrap the image in Compose or another supervisor. Before starting it:

1. Review the image build arguments and dependency versions.
2. Mount the configuration file and persistent state directories.
3. Provide keys and tokens through the deployment environment or a secret store.
4. Confirm that the container network can resolve both chain endpoints.
5. Verify that host ports do not collide with another relayer instance.

Typical lifecycle commands are:

~~~text
docker compose build
docker compose up -d
docker compose logs -f <service>
docker compose ps
docker compose stop <service>
~~~

The container image sets a larger Rust thread stack because proof-generation and storage code can use deep call stacks. Preserve that setting when translating the container command into another supervisor.

Run one relayer per container when possible. This keeps restart, resource accounting, log routing, and persistent directories independent. Multiple logical relayers in one configuration are supported where the selected binary implements them, but they still need separate monitoring and state ownership.

## HTTP management API

The HTTP server is an operator interface, not a public RPC service. Bind it to a private interface or place it behind an authenticated network boundary. Requests must include the configured X-Token header.

The available routes are:

| Route | Method | Purpose |
| --- | --- | --- |
| /relay_messages | POST | Ask the Gear-to-Ethereum path to relay specified messages |
| /relay_transactions | POST | Ask the Ethereum-to-Gear path to relay specified transaction hashes |
| /get_merkle_root_proof | POST | Return proofs for requested finalized blocks |

The route is asynchronous where the operation may take time. A successful request generally means that work was accepted or queued; it does not mean that the destination transaction is finalized.

Example proof request:

~~~text
curl -sS http://127.0.0.1:8080/get_merkle_root_proof \
  -H 'Content-Type: application/json' \
  -H 'X-Token: <configured-token>' \
  -d '{"blocks":[12345]}'
~~~

The proof response contains the block number, root, proof, and related metadata needed by the configured submission path. Treat the response as versioned implementation output: validate it against the running binary before building an external automation contract around field names.

Typical HTTP failures:

| Status | Meaning |
| --- | --- |
| 401 | Missing or incorrect X-Token |
| 400 | Malformed JSON or invalid request shape |
| 404 | Route is not available in the selected process |
| 500 | The request reached the relayer but the operation failed |
| 202 | Work was accepted for asynchronous handling |

Do not expose this API directly to the Internet. The token is an authentication check, not a replacement for TLS, network isolation, rate limiting, or audit logging.

## Manual relay and recovery

Use gear-eth-manual or eth-gear-manual only for an explicitly reviewed recovery or migration procedure. Record the source transaction, destination account, operator, reason, and resulting chain transaction.

Before manual relay:

- Check whether the automated relayer already submitted the item.
- Check the durable state and destination contract state.
- Confirm that the message is not being retried by another process.
- Confirm the nonce, queue, and destination network.
- Capture the expected fee and signer account.

After manual relay:

- Record the destination transaction hash.
- Verify the transaction receipt and event.
- Reconcile the local relay state.
- Remove or quarantine any stale retry item only after confirming the destination state.

The kill-switch process is the emergency control for stopping relay activity. It does not repair proofs, roll back chain state, or replace a transaction already accepted by a destination contract. Use it to stop new work while the incident is investigated.

## Queue cleaning

queue-cleaner removes queue entries that have passed the configured expiry or delay policy. Run it only against the intended queue and network. A cleaner must not be treated as a generic database garbage collector: queue entries can represent messages whose delivery and dispute windows have protocol meaning.

Recommended procedure:

1. Stop or pause normal relay work for the affected queue if the run is corrective.
2. Confirm the target network and queue address.
3. Dry-run or inspect the command help if the version supports a dry-run mode.
4. Capture the candidate entries and the configured expiry.
5. Run the cleaner.
6. Verify queue length, events, and relayer state.
7. Resume relay and watch retries.

## Root proof retrieval

The fetch-merkle-roots utility is useful when the operator needs to retrieve a root proof without starting a full relay loop. It should use the same network genesis, Gear endpoint, authority-set context, and proof-storage conventions as the relayer that will consume the proof.

A retrieved proof is useful for:

- Inspecting a failed submission.
- Replaying a submission in a controlled environment.
- Comparing proof bytes produced by two revisions.
- Supplying a manually reviewed recovery transaction.

A proof fetched from one network or genesis configuration must not be submitted to another. Keep the block number, root, proof format, circuit version, and source binary together in the incident record.

## Updating the Solidity verifier

update-solidity-verifier generates or installs the Ethereum verifier output associated with the configured proof circuit. Treat generated verifier files as build artifacts. Do not hand-edit generated Solidity to solve a deployment problem.

Before updating:

- Pin the prover and gnark-wrapper revisions.
- Preserve the current verifier address and deployment transaction.
- Confirm the generated circuit public-input order.
- Generate the verifier in a clean output directory.
- Compare the generated source and hashes with the expected release artifact.
- Deploy first on a test network.
- Verify that MessageQueue accepts a known-good root proof and rejects a modified proof.

The verifier update changes a cryptographic trust boundary. It should be reviewed and deployed separately from routine relayer restarts.

## Monitoring

At minimum, monitor:

- Process liveness and restart count.
- Gear and Ethereum RPC latency, errors, and reconnects.
- Finalized block height observed by each direction.
- Oldest unrelayed message and oldest pending root.
- Proof-generation duration and failures.
- Destination transaction submission, replacement, and receipt status.
- Queue size and expired-message count.
- Available disk space for storage and proof artifacts.
- Memory and CPU during proof generation.
- HTTP authentication failures and unexpected management requests.

Alert on lack of progress, not only process exit. A relayer can remain alive while its source subscription, proof worker, signer, or destination transaction path is stalled.

## Common failure patterns

### The process starts and exits immediately

Check configuration parsing, network genesis, key parsing, filesystem permissions, and whether another service already owns the configured port. Run the selected command in the foreground and increase RUST_LOG before changing deployment files.

### The relayer sees blocks but does not submit

Check finality policy, queue discovery, root priority, signer balance, destination contract address, and whether an earlier proof or transaction is still pending. For root relaying, inspect both the root lifecycle state and proof-storage artifacts.

### Proof generation fails

Check the exact block and authority-set context, proof-storage contents, native prover dependencies, memory, thread stack, and circuit/prover version. Do not reuse an artifact produced for a different genesis or circuit version.

### Transactions are submitted but messages remain pending

Check the destination receipt, contract event, message nonce, queue address, processing delay, and whether the message proof is valid. A submitted transaction is not the same as a processed message.

### Restart changes behavior

Check whether the process recovered its JSON state and whether a .bak file or interrupted atomic write is present. Compare the recovered block/root cursor with chain state. Preserve both the state file and its backup before attempting repair.

## Safe upgrade procedure

Use a rolling procedure for independent relayers:

1. Stop one instance cleanly.
2. Preserve its configuration, durable state, proof directory, and logs.
3. Deploy the new binary or image.
4. Start it in foreground or with a tightly watched supervisor.
5. Confirm source progress and destination submission.
6. Compare metrics and logs with the previous version.
7. Continue with the next instance only after the first is healthy.

Do not upgrade the prover, gnark wrapper, Solidity verifier, and relayer configuration simultaneously unless the release procedure explicitly couples them. If a circuit or verifier changes, treat the deployment as a protocol upgrade and retain the old artifacts for replay and incident analysis.

## Release checklist

Before declaring the bridge healthy:

- Every endpoint resolves to the expected network.
- Genesis values and contract addresses match the deployment manifest.
- Keys are loaded from the intended secret source.
- State and proof paths are persistent and writable.
- No two relayers share a mutable state path.
- Management endpoints require the expected token.
- Metrics are reachable from the monitoring system.
- A known source block has been observed.
- A known test message or root has been traced through submission and receipt.
- Alerts fire for a stopped or stalled worker.
- Logs include the process name, network, and relayer identity.
