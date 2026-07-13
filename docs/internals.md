# Bridge internals

This page describes the current runtime composition of the bridge. The implementation is split into asynchronous services connected by channels; most services persist a cursor or work item before handing it to the next stage.

## End-to-end map

The core Gear-to-Ethereum path is:

~~~text
finalized Gear justification
        |
        v
Gear BlockListener
        |
        +--> MerkleRootStorage (blocks, inclusion proofs, root state)
        |
        +--> MerkleRootRelayer
                |
                +--> AuthoritySetSync -> proof storage
                |
                +--> FinalityProver / SharedFinalityProver
                |
                +--> MerkleRootSubmitter -> Ethereum MessageQueue
                |
                +--> authenticated HTTP proof requests
~~~

The Ethereum-to-Gear core path is separate:

~~~text
Ethereum beacon RPC
        |
        v
ethereum_checkpoints::Relayer
        |
        v
checkpoint-light-client program on Gear
        |
        v
eth-events-* / historical-proxy consumers
~~~

Token relayers sit above these protocol primitives. They turn verified message/event evidence into token-manager calls; they are not the component that proves a Gear message-queue root.

## Process bootstrap

[relayer/src/main.rs](../relayer/src/main.rs) creates:

- a global Rayon pool for circuit work;
- a multi-thread Tokio runtime with a blocking-thread limit;
- dotenv loading and module-specific logging;
- the Clap command dispatcher.

gear-eth-core validates its command-line arguments and environment values, then starts one core relayer.

For each core relayer, start_gear_eth_core_relayer:

1. Connects an ApiProvider to the configured Gear endpoint.
2. Creates the Ethereum signer client for the configured MessageQueue and fee payer.
3. Creates either filesystem or Gear-backed proof storage.
4. Creates MerkleRootStorage for block/root/submission state.
5. Binds the relayer HTTP listener.
6. Creates the finality prover and authority-set synchronizer used by the relayer.
7. Registers Prometheus collectors.
8. Starts the API provider and the relayer task.

The expensive proof work is isolated behind channels so block listening, scheduling, proving, and submission can make progress independently. The process owns one set of clients, storage, HTTP routes, and scheduler state for this command invocation.

## Gear finality and block delivery

[relayer/src/message_relayer/common/gear/block_listener.rs](../relayer/src/message_relayer/common/gear/block_listener.rs) consumes GRANDPA justifications rather than arbitrary best-head blocks. For each finalized block it:

1. Fetches the block and converts it to the bridge's GearBlock representation.
2. Extracts whether the message queue root changed and whether an authority set changed.
3. Produces and stores the raw block-inclusion/finality material in MerkleRootStorage.
4. Broadcasts the block to the root relayer and authority-set synchronizer.

The listener has a large broadcast capacity because proving and era synchronization can lag behind block production. It replays unprocessed state at startup, detects gaps in live justifications, and replays missing ranges. On recoverable provider errors it reconnects and starts replay from the last finalized cursor.

The block storage is a source of recovery, not just a cache. A consumer that falls behind can be restarted from the persisted block set. A broadcast lag warning is therefore different from a proof being lost; operators should inspect persisted state before deleting anything.

## Gear-to-Ethereum root flow

The root relayer is implemented in [relayer/src/merkle_roots/mod.rs](../relayer/src/merkle_roots/mod.rs).

### 1. Detect a new root

MerkleRootStorage recognizes the Gear bridge's QueueMerkleRootChanged event and records queue id and root for the block. It also records message nonces, authority-set changes, and the raw inclusion proof needed later by the circuit.

The root is keyed locally by the pair of Gear block number and root hash. The block number is not sufficient by itself because a root may be retried or compared against an already stored value.

### 2. Resolve the authority-set proof

A final proof must show both the message-queue root and the authority set that finalized its block. The root relayer asks proof storage for the proof corresponding to the authority-set id that signed the block.

If the proof is absent, the root enters WaitForAuthoritySetSync. AuthoritySetSync obtains the required finalized era transition, composes a recursive proof from the configured genesis authority set, and stores the resulting proof. Waiting blocks are released after the authority-set response arrives.

GenesisConfig is a compatibility boundary: its authority-set id and hash are fixed inputs to the recursive proof chain. They must match the deployed verifier and the history from which proof storage was initialized.

### 3. Schedule final proof generation

Once the inner authority-set proof is available, the root is recorded as GenerateProof and sent to a finality-prover channel. The request contains:

- Gear block number and hash;
- queue id and root;
- the authority-set proof;
- raw block-inclusion material;
- whether the request may be batched;
- the relayer-specific context needed to reconnect to Gear and invoke the prover.

Normal block traffic is batchable. Non-batched requests are used for catch-up, critical-threshold recovery, supervisor checks, and authenticated HTTP requests that need a specific proof quickly.

### 4. Compose and generate the proof

The Rust prover builds a recursive Plonky2 proof that the root is present in bridge storage and that the containing block is finalized. The relayer then passes the exported circuit data to the Go gnark wrapper, which produces the BN254 Plonk proof serialized for Solidity. See [circuits and proofs](circuits-and-proofs.md).

### 5. Submit and confirm on Ethereum

MerkleRootSubmitter sends submitMerkleRoot(blockNumber, merkleRoot, proof) to the configured MessageQueue. It records local submission states (pending, broadcast, confirmed, or failed), waits for the configured number of confirmations, and checks the finalized on-chain root before reporting success.

The submitter reconciles recovered work against Ethereum before broadcasting again. This matters after a process crash between transaction broadcast and local state persistence: local broadcast state is not treated as proof that the root finalized.

After a successful confirmation, the root transitions to Finalized; waiting HTTP requests receive the proof response. A failed transaction transitions to Failed and is surfaced in logs and metrics.

## Root scheduler and priority behavior

The root relayer keeps a pending batch with timestamps and message-nonce counts.

- spike_window removes old queue timestamps from spike calculations.
- spike_threshold causes immediate proof generation when the total number of message nonces in the current batch reaches the threshold.
- spike_timeout flushes an ordinary batch after its timeout.
- priority_spike_timeout flushes a batch containing a priority request sooner.
- A bridging_payment_address enables priority handling for recognized priority-payment events.
- critical_threshold forces a non-batched proof when the last confirmed root is too far behind. authority_set_change is an alternative trigger that forces a proof around an authority-set transition.
- An authenticated /get_merkle_root_proof request is handled as a priority, non-batched request and may use a fresh justified block while the requested block is being caught up.

A supervisor tick periodically reads the latest Gear queue root and the corresponding Ethereum root. If Ethereum has no matching root, or a configured critical threshold is reached, it schedules a recovery proof. It deduplicates a root while the same proof is in flight.

The prover gives non-batched requests priority over ordinary batches. Batches are grouped by authority-set id and queue id, and responses are sent back through the channel associated with the originating request.

## Ethereum-to-Gear core

[relayer/src/ethereum_checkpoints/](../relayer/src/ethereum_checkpoints/) contains the core Ethereum-to-Gear relayer. The eth-gear-core command creates a beacon client, a Gear client, and a checkpoint-light-client relayer using:

- the checkpoint-light-client program id;
- an Ethereum beacon RPC endpoint;
- a Gear signer URI and endpoint;
- a slot-batch multiplier.

The relayer follows finalized beacon-chain data and submits sync-committee/finality updates to the Gear light-client program. Token/event consumers can then ask the Gear-side eth-events-* programs and historical-proxy to verify a transaction or event.

This process is distinct from the Ethereum token relayer: one advances the light-client state, while the other submits user/event receipts through the programs that consume that state.

## Token-relayer internals

### Gear to Ethereum

The implementations under [relayer/src/message_relayer/gear_to_eth/](../relayer/src/message_relayer/gear_to_eth/) compose several workers:

- Gear block listeners and message-queued/message-paid event extractors;
- an Ethereum root extractor;
- an accumulator for roots and message indexes;
- Gear Merkle-proof and message-data fetchers;
- a transaction/status sender;
- optional paid-message filtering and HTTP ingestion.

The all-transfer and paid-transfer variants share the protocol evidence pipeline but differ in which messages they admit and how bridging-payment requests are selected. A paid transfer also has an HTTP server used to receive the payment/relay work described by the command's web-server options.

The final Ethereum transaction is not accepted merely because a message nonce exists. The token relayer waits for the relevant root, requests the message inclusion proof, and submits a MessageQueue processing call with that proof.

### Ethereum to Gear

The implementations under [relayer/src/message_relayer/eth_to_gear/](../relayer/src/message_relayer/eth_to_gear/) monitor finalized Ethereum blocks, extract deposits or paid-transfer events, compose event proofs, and send a receipt to a Gear receiver program. They persist Ethereum blocks/transactions so a restart can replay unprocessed work.

The beacon/light-client path and the event/message path are complementary:

1. eth-gear-core advances checkpoint-light-client state.
2. The token relayer asks historical-proxy/eth-events-* to prove an event against that state.
3. The VFT manager mints, unlocks, or transfers the corresponding Gear-side token.

## HTTP routing and channels

The shared HTTP server in [relayer/src/server.rs](../relayer/src/server.rs) uses the X-Token header for authentication. It exposes only the routes enabled by the caller:

- /get_merkle_root_proof sends block numbers to the owning root relayer;
- /relay_messages sends Gear message descriptors to a Gear-to-Ethereum token relayer;
- /relay_transactions sends Ethereum transaction hashes to an Ethereum-to-Gear token relayer.

The server deduplicates repeated items within one request. It returns 401 for a missing or incorrect token, 200 when all items were accepted/handled, 202 for partial acceptance, and 500 when no item could be queued or a response channel failed.

The selected process creates one listener and one channel for its configured relayer. The port is therefore the routing boundary; the JSON request does not carry a network name.

## Persistence and failure boundaries

The main persistent boundaries are:

- Merkle-root JSON state, including blocks, roots, and Ethereum submission states.
- Proof storage, either filesystem files or a Gear proof-storage program/config directory.
- Ethereum block/transaction storage for token relayers.
- The gnark proving data directory, containing SRS and generated key material.

The root relayer saves atomically and keeps a backup. The block listener writes block evidence before broadcasting it. The submitter checks finalized Ethereum state before retrying recovered work. These boundaries provide idempotency, but they do not make arbitrary deletion safe.

RPC errors are classified at the provider and listener boundaries. Recoverable transport/subscription errors trigger retries and reconnects; permanent errors or exhausted retry policies are returned to the owning service. When a service channel closes, the parent relayer treats that as a component failure rather than silently continuing with incomplete proof or submission state.

## Module map

| Concern | Main implementation |
| --- | --- |
| CLI and process supervision | [relayer/src/main.rs](../relayer/src/main.rs), [relayer/src/cli/](../relayer/src/cli/) |
| CLI and environment configuration | [relayer/src/cli/](../relayer/src/cli/) |
| Gear finalized-block delivery | [relayer/src/message_relayer/common/gear/block_listener.rs](../relayer/src/message_relayer/common/gear/block_listener.rs) |
| Root state machine | [relayer/src/merkle_roots/mod.rs](../relayer/src/merkle_roots/mod.rs) |
| Root persistence | [relayer/src/merkle_roots/storage.rs](../relayer/src/merkle_roots/storage.rs) |
| Authority-set proving | [relayer/src/merkle_roots/authority_set_sync.rs](../relayer/src/merkle_roots/authority_set_sync.rs) |
| Final proof worker | [relayer/src/merkle_roots/prover.rs](../relayer/src/merkle_roots/prover.rs), [relayer/src/prover_interface.rs](../relayer/src/prover_interface.rs) |
| Ethereum root submission | [relayer/src/merkle_roots/submitter.rs](../relayer/src/merkle_roots/submitter.rs) |
| Ethereum beacon/light-client relay | [relayer/src/ethereum_checkpoints/](../relayer/src/ethereum_checkpoints/) |
| Gear-to-Ethereum token relay | [relayer/src/message_relayer/gear_to_eth/](../relayer/src/message_relayer/gear_to_eth/) |
| Ethereum-to-Gear token relay | [relayer/src/message_relayer/eth_to_gear/](../relayer/src/message_relayer/eth_to_gear/) |
| Solidity verifier and MessageQueue | [ethereum/src/VerifierMainnet.sol](../ethereum/src/VerifierMainnet.sol), [ethereum/src/VerifierTestnet.sol](../ethereum/src/VerifierTestnet.sol), [ethereum/src/MessageQueue.sol](../ethereum/src/MessageQueue.sol) |
