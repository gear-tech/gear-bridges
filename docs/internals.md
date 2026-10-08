# Bridge internals

This page describes the current runtime composition of the bridge. The implementation is split into asynchronous services connected by channels; most services persist a cursor or work item before handing it to the next stage.

## End-to-end map

### BEEFY/MMR Hoodi lane

This isolated test lane replaces the outbound ZK root-verification boundary. It retains the public legacy deployment and the separate inbound Beacon path.

~~~text
BABE authoring + GRANDPA source finality
                  |
Gear VFT burn -> GearEthBridge queue at B
                  |
       MMR insertion at B+1, queue snapshot + next authorities
                  |
       native BEEFY commitment at C
                  |
 independent follower -> BeefyClient -> VaraQueueRootVerifier
                  |                         |
                  +--- root publisher -> MessageQueue
                                            |
                              paid worker -> ERC20 release

Hoodi ERC20 lock -> finalized EL receipt + Beacon proof
                  |
 checkpoint worker -> Gear checkpoint-light-client
                  |
 inbound worker -> HistoricalProxy / Electra event verifier
                  |
            VFT manager -> Gear VFT mint
~~~

#### Source commitment and wire contract

The isolated [Vara runtime](../../source/vara/runtime/vara/src/lib.rs) configures `pallet_mmr` with `Keccak256`, `MmrLeaf` and `DepositBeefyDigest`; `pallet_beefy` uses `MmrLeaf` for validator updates and ancestry. [`bridge_leaf::VaraBridgeProvider`](../../source/vara/runtime/vara/src/bridge_leaf.rs) commits the parent timestamp and queue state before delayed queue clearing. BABE authors blocks and GRANDPA finalizes the source chain; BEEFY authenticates MMR commitments for the bridge.

[`QueueSnapshot`](../tools/beefy-relay/src/protocol.rs) and the runtime provider encode exactly `version=2:u8 | "vara":4 | bridgeDomain:32 | timestamp:u64LE | initialized:u8 | queueId:u64LE | root:32`: 86 bytes. The outer MMR leaf is version 0 and 113 SCALE bytes. The inherited Solidity `parachainHeadsRoot` field in [`BeefyClient`](../ethereum/src/beefy/BeefyClient.sol) contains the snapshot hash, not parachain heads.

The domain is Keccak256 of `vara/gear-eth-bridge-domain/v2`, `sourceDomain32`, the destination chain ID as 32-byte big-endian, and `queueAddress20`, concatenated in that order. Source `BridgeDomain` configures one lane, not a destination registry. Deployment and relay evidence separately pin genesis identity.

For queue source block B, insertion L=B+1, MMR start S and commitment C, the leaf index is L-S and the leaf count is C-S+1; B<C and B>=S are required. [`Source::proof`](../tools/beefy-relay/src/source.rs) reads historical state and checks the initialization boundary, parent identity, insertion-time next authorities, leaf coordinates and signed root. Reuse it rather than reconstructing a snapshot from current state. Archive state and offchain MMR nodes are required.

Rust preflights bound SCALE lengths before decoding: at most 256 authorities, 64 payload entries, 64 KiB of payload bytes, 82,688 signed-proof bytes, one 113-byte MMR leaf and 256 MMR proof items. Hex and justification byte arrays are bounded before byte allocation; the existing WebSocket transport limits responses to 10 MiB before JSON decoding. Accepted SCALE encodings must consume their entire input canonically.

#### Verification, publication and accounting

The [EVM relay](../tools/beefy-relay/src/ethereum.rs) calls `createFiatShamirFinalBitfield` and `submitFiatShamir`. The client also retains interactive verification. Bootstrap trusts the configured current/next authority sets. Each update authenticates the newest leaf's parent timestamp; a next-set commitment hands over authority using that leaf's next-set proof. [`VaraQueueRootVerifier`](../ethereum/src/VaraQueueRootVerifier.sol) binds queue proofs to the client's latest accepted MMR anchor and the destination-bound snapshot.

A canonically mined commitment is capacity/progress evidence, not finalized acceptance. The [follower and root publisher](../tools/beefy-relay/src/tokens.rs) retain original source, client, anchor, nonce, raw bytes, transaction hash and inclusion. A signed root publication cannot be reanchored; finality promotion may add evidence but cannot replace that identity. [`MessageQueue`](../ethereum/src/MessageQueue.sol) retains maturity, message inclusion, global nonce replay protection, custody dispatch and historical-root redemption. Empty initialized queue progress creates no root and does not restart maturity. Bounded campaign acceptance requires canonical finalized commitment, root and release receipt evidence; this is not an additional on-chain dispatch guard. The paid worker uses the configured confirmations and maturity checks.

The corrected common queue preserves legacy slots 0–13: block→root at 11, root→timestamp at 12 and nonce→processed at 13. Appended slots 14–16 hold recovery, the BEEFY root floor and block→timestamp. `getMerkleRootTimestamp(bytes32)` remains the historical API; `getMerkleRootTimestampForBlock(uint256)` and redemption share the effective timestamp reader. Per-block time takes precedence; legacy fallback is permitted only before activation or below the floor. Missing applicable timestamps fail closed. Repeated hashes mature independently; authenticated conflicts clear only the affected block. The 300/3600-second delays and historical redemption remain intact.

The [checkpoint worker](../relayer/src/ethereum_checkpoints/) updates the [checkpoint-light-client](../gear-programs/checkpoint-light-client/) independently. The [inbound worker](../relayer/src/message_relayer/eth_to_gear/) supplies finalized receipt/Beacon proofs through [HistoricalProxy](../gear-programs/historical-proxy/app/src/service.rs) and the [Electra verifier](../gear-programs/eth-events-electra/) to the [VFT manager](../gear-programs/vft-manager/app/src/services/submit_receipt/mod.rs). This path remains separate from BEEFY. The [paid outbound worker](../relayer/src/message_relayer/gear_to_eth/) redeems verified messages for ERC20 release.

One supervised follower/root owner serves this deployment; never run `gear-eth-core` or a second root publisher against its queue. Follower, root publisher, paid worker and campaign use distinct EVM signers. Checkpoint, inbound, campaign, governance and rotation retain their established Gear roles. The [campaign](../tools/beefy-relay/src/tokens_soak.rs) observes independent mint/root proof handoffs and must not provide its own successful proofs. The private wrapper holds the campaign lock and passes it to the child; Rust `tokens_soak` itself does not hold that lock. Operational executable copies stay immutable outside Cargo target. Same-user malicious file replacement is not qualified.

#### Snowbridge lineage and deployment boundary

[Interop fixtures](../tools/beefy-relay/src/fixtures.rs) pin Snowbridge commit `1201293e482ef052b9c3989dcf680046704fef3d`. This lane differs through a direct Vara queue snapshot instead of BridgeHub/parachain proofs, destination binding, authenticated source freshness, tiny-set quorum and existing Vara queue/recovery semantics. Retain `MAX_VALIDATORS=256` and both signature ceilings at 86, capped by `floor(N/3)+1` with native small-set quorum rules. N=59 requires 20 EVM signatures and native quorum 40; N=256 requires 86 and 171 respectively. Both EVM paths require distinct positions and accounts; authenticated native rosters require distinct source keys and derived EVM accounts. N=2 requires both signatures. These bounds and synthetic fixtures do not qualify the actual production roster or transfer upstream audit conclusions.

The architectural reference for this finalization is Snowbridge `630c2081359e2417496e7f4c1a189a7eb40f7bd4`; the older interoperability-vector pin above remains distinct. Neither reference transfers an upstream audit to these adapters.

`Source::connect_pair` authenticates both RPCs at one common finalized upgraded pin: runtime/API identity, separately labeled source `:code` hashes, current/next authorities, activation `G`, actual first insertion `S`, MMR root/count and destination-bound domain. The first-leaf proof establishes insertion geometry; activation is not read from block zero and missing activation is HOLD. New anchors retain `domainBindingBlock` and `sourceIdentity`; old anchors remain restricted to the explicit legacy runtime rather than silently attaching to a normal or production deployment. Public artifact, validator-roster, weight, custody and migration qualification remain separate gates. The local 3-of-5 Safe is test-only, not independent recovery authority. Never upgrade a retained test queue with block-keyed slot 12 to the corrected root-keyed layout.

### Legacy ZK lane

The legacy core Gear-to-Ethereum path is:

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

Bootstrap requires an independently trusted recent checkpoint root, not the deployment provider's choice of finality. `checkpoints-tool` requires that root and the trusted genesis validators root; the actor checks bootstrap proofs against the supplied root and rejects period-zero bootstrap. The selected `Network` fixes genesis validators identity, genesis time and the Deneb/Electra/Fulu schedule for the lifetime of the actor.

Regular updates and replay enforce `current_slot >= signature_slot > attested_slot >= finalized_slot`, sequential authenticated committee transitions and the fork at the preceding signature slot. Replay headers must already be strictly ordered and chain to the authenticated interval; neither the relayer nor the actor sorts malformed input into validity. Revision compare-and-set, bounded checkpoint history and native BLS verification remain enforced.

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

The final Ethereum transaction is not accepted merely because a message nonce exists. The token relayer waits for the relevant root, requests the message inclusion proof, and submits a MessageQueue processing call with that proof. Confirmed Ethereum roots are persisted before the scanner advances its cursor. Finalized Gear queued and paid events have separate block-hash cursors; their observations and pending pairs survive lag and restart until the outgoing transaction completes.

Completion requires the original signed Ethereum transaction and its canonical finalized successful receipt. Match the configured queue's exact `MessageProcessed` nonce/hash and, for token-manager dispatches, the exact `ERC20Manager.Bridged` token, sender, receiver and raw amount in the same receipt/queue segment. A processed nonce, a competing transaction, a reverted replay or missing original receipt cannot complete the operation. Completed journal entries are requalified at startup; preserve original signed bytes, account nonce, message identity and discovery cursors. Generic application routes and paid-message eligibility remain separate from token-effect checks.

### Ethereum to Gear

The implementations under [relayer/src/message_relayer/eth_to_gear/](../relayer/src/message_relayer/eth_to_gear/) monitor finalized Ethereum blocks, compose proofs and retain the original signed Gear dispatch, receipt bytes, request/reply linkage and per-log consumer effects. `Processed` alone never completes an entry. Completion requires the exact canonical finalized original proxy/consumer reply and every corresponding authenticated settlement row; missing history, unknown outcomes and arbitrary inner errors HOLD without replacing the proof transaction. An original `NativeSettlementPending` result can advance only through its original wrapper payout becoming `Delivered` with zero returned value, followed by non-economic `ReconcileReceipt` and finalized `Settled` rows/`Processed` receipt state. That continuation has its own durable signed identity; it cannot mint, transfer, redeem or rewrite the original reply.

Receipt verification consumes the complete SCALE frame and receipt RLP, requires a successful EIP-658 status, and checks exact SSZ branch depth. Historical headers must ascend within `(receipt.slot, checkpoint.slot]`, connect through parent roots and terminate at the exact authenticated checkpoint; empty ancestry is valid only at that checkpoint. Receipt endpoints enforce the immutable network fork schedule. The historical proxy authenticates the requested slot and verifier reply but preserves the consumer's raw reply bytes: outer proxy success does not establish a successful inner application result or economic settlement.

Ordinary Gear-origin returns use escrow `TransferFrom`. Native policy is explicit and snapshotted per deposit: the manager-authorized wrapper burns manager escrow and dispatches native value directly, retaining the original payout child and returned-value obligation. Generation/lease ownership and per-log child outcomes prevent an expired wait from dispatching the economic effect again. Queued, returned or ambiguous native value is not settlement. Paused/admin source reconciliation also requires the original request/child/hash and an authenticated builtin outcome; it never infers “not queued” from a timeout.

SDK callers supply an immutable, independently approved `InboundProofProfile` binding both network identities, the source runtime and block, actor CodeIds, exact IDL digests, endpoint framing, checkpoint network, forks and consumer route. Frontend completion uses `onFinalized`, not an outer proxy success flag. Outbound SDK callers supply the exact expected effect from the packed original source message. Missing profiles or deployment configuration render HOLD before signing.

The indexer validates canonical finalized batches before effects or cursor advancement. Ethereum completion joins exact queue and token effects in one original receipt; Gear relay events alone are transport evidence. `ReceiptDepositSettled` rows correlate slot, transaction index and receipt-local log index, preserving the existing first-log identity and separating later deposits. The offline `1791244800000-receipt_identity.js` migration adds receipt identity/settlement journals; it is not executed by these source changes. Do not use legacy completion rows as new settlement evidence without requalification.

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
