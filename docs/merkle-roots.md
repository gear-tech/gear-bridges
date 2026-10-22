# Merkle roots

A Gear message-queue root commits to queued messages. Ethereum authenticates that root through the BEEFY client and a canonical MMR queue-snapshot proof, then verifies individual-message membership separately. The active outbound pipeline is **commitment follower → MMR root publisher → retained message delivery**. Updating this source does not activate a runtime, migrate public contracts/programs or approve a bootstrap.

This page describes the existing publisher, journals and recovery boundaries. See [Running the bridge](running-the-bridge.md) for env/bootstrap/profile procedures and [circuits and proofs](circuits-and-proofs.md) for proof boundaries. The independent Ethereum→Gear Beacon proof-composition path is unchanged.

## What a root represents

The Gear bridge pallet accumulates messages in a binary Merkle trie. A finalized queue-root observation has a source block number/hash, queue identity and root. Publication authenticates the historical bridge snapshot under an MMR leaf covered by an accepted BEEFY commitment. Source genesis/domain, destination chain/queue, MMR boundary, snapshot height and anchor must match the reviewed deployment.

A queue-root proof is not an individual-message proof. `MessageQueue.submitMerkleRoot(sourceBlock, root, proof)` authenticates and records the root; `MessageQueue.processMessage(...)` later checks the message's binary Merkle proof, replay protection and maturity. Publication alone is not transfer completion.

The active implementation is in [follower.rs](../tools/beefy-relay/src/follower.rs), [root_publication.rs](../tools/beefy-relay/src/root_publication.rs), [source.rs](../tools/beefy-relay/src/source.rs) and [protocol.rs](../tools/beefy-relay/src/protocol.rs). On-chain boundaries are [BeefyClient.sol](../ethereum/src/BeefyClient.sol), [VaraQueueRootVerifier.sol](../ethereum/src/VaraQueueRootVerifier.sol) and [MessageQueue.sol](../ethereum/src/MessageQueue.sol).

## Commitments, inventory and publication

`beefy-relay tokens-follow` authenticates two independent source endpoints against the original manifest/bootstrap, source binding and ordered authority history. It advances the Ethereum client using signed BEEFY commitments, retaining original transaction evidence for each submission. Authority handovers, heartbeat/client-expiry requirements and root demand remain authenticated even when root publication is held.

Held, already-covered publications retry after a short maintenance window at
completed source polls or same-set commitment boundaries. This does not cancel
source work or bypass mandatory handovers and client heartbeats.

The follower inventories historical source progress starting at immutable `beefyRootMinimum`, not first publication or maximum progress. Later roots or empty progress cannot satisfy an older missing nonzero queue epoch. A frozen demand batch may share a BEEFY update; it does not eliminate each inventoried publication.

For each registered target, the publisher:

1. Authenticates the exact historical source/witness snapshot and root/progress kind.
2. Selects a covering accepted anchor and builds the canonical v2 MMR queue-proof envelope.
3. Persists publication identity, proof/anchor and reserved account nonce before broadcast; signed bytes/hash are durable before sending.
4. Reconciles the original transaction and authenticated finalized destination state.
5. Records original acceptance, authenticated external observation, pending uncertainty or a proven failed original without confusing these outcomes.

Missing or pruned source/history evidence is HOLD, not an empty root or permission to skip an epoch. Bounded-distance catch-up uses the actual initialized snapshot: nonzero snapshots publish roots; zero snapshots use authenticated empty progress. Each newly backfilled root has its own maturity delay.

## Commands and ownership

The continuous actor uses the approved manifest, protected commitment/root wallet files and original private output directory:

~~~sh
target/release/beefy-relay tokens-follow \
  --source-rpc "$SOURCE_RPC" --witness-rpc "$WITNESS_RPC" \
  --ethereum-rpc "$EL_RPC" --wallet "$FOLLOWER_WALLET" --root-wallet "$ROOT_WALLET" \
  --deployment-manifest "$DEPLOYMENT_MANIFEST" --output-dir "$FOLLOWER_DIR" \
  --client-deployment-transaction "$CLIENT_DEPLOYMENT_TRANSACTION"
~~~

The optional original client CREATE locator does not approve its bootstrap: canonical signed creation, ancestry, receipts and constructor bindings are still authenticated. Public Ethereum defaults to Hoodi. Mainnet requires explicit `--ethereum-network mainnet` and the independently reviewed retained-state mainnet manifest. Approved owned fixtures use the existing `--owned-fixture-network` descriptor and loopback restrictions; a rehearsal cannot be relabeled as a public actor.

`tokens-follow` holds the deployment lock across following and root publication. Stop that owner before a reviewed standalone resume. The standalone command requires an existing follower journal and exactly one original registration matching the target and registered output path:

~~~sh
target/release/beefy-relay tokens-publish-root \
  --source-rpc "$SOURCE_RPC" --witness-rpc "$WITNESS_RPC" \
  --ethereum-rpc "$EL_RPC" --wallet "$ROOT_WALLET" \
  --deployment-manifest "$DEPLOYMENT_MANIFEST" \
  --publisher-state "$FOLLOWER_DIR/state.json" \
  --registration "$ORIGINAL_ROOT_REGISTRATION" \
  --output "$ORIGINAL_REGISTERED_PUBLICATION_PATH"
~~~

Use retained registration evidence and the exact path recorded under the follower's `root-publications/`; do not manufacture a new registration, select an arbitrary block, move the output or replace held journals. The command reconciles until original `Accepted` or authenticated external `Observed`; the latter does not declare an unresolved local original successful.

Use distinct Ethereum fee-payers for commitment submission, root publication, every independent delivery/manual process and observer. One process's nonce lock cannot protect another. Never run independent submitting processes on one account, active-active journal writers or a standalone publisher concurrently with the deployment owner. Deployment and governance keys remain separate.

## Ethereum verification and delivery

`submitMerkleRoot(uint256,bytes32,bytes)` receives the original source height/root and canonical v2 proof, not packed field elements or raw MMR siblings. The queue checks its configured BEEFY floor and source bounds, calls the typed verifier and records the root/timestamp or applies its existing conflict/emergency behavior. Invalid queue proofs reject with `InvalidQueueRootProof`; challenge/emergency conditions are protocol holds, not generic transport retries. Anyone may submit a valid proof; signer identity is an operational ownership boundary, not proof authority.

`submitEmptyQueueProgress` authenticates a zero-root snapshot and advances allowed progress without creating a redeemable root or resetting any maturity timestamp. Empty progress does not establish an earlier missing root.

Retained Gear→Ethereum token/manual workers consume finalized publications, retain exact historical roots and verify individual-message membership. Delivery still requires queue maturity, the original signed transaction's canonical finalized successful receipt, confirmation policy, exact queue event and same-receipt token effects. A processed bit or another publisher's root is not original transaction success. Native pending/mailbox value is not economic settlement.

The retained root extractor owns `ethereum_roots.json`, binding observations to finalized block hashes and transaction/log coordinates. Subscriptions only wake finalized catch-up. Its bounded cache uses publication order, so a newly backfilled historical root can remain usable even when its source height is old. Preserve historical execution headers and source archives; unavailable finality evidence cannot advance discovery.

## Durable state and uncertainty

Back up the original deployment/bootstrap records, follower `state.json`, all registered `root-publications/` intents, terminal submissions, authenticated execution proof cache and delivery discovery/transaction journals together. Use private stable paths and exclusive ownership. Do not transplant journals to another network, signer, deployment or rehearsal mode.

Follower schema 4 distinguishes own submissions from authenticated external accepted transactions. Root-intent schema 4 distinguishes `observedProgress` from the original `publicationReceipt`. A matching current root/checkpoint or a log alone is insufficient: accepted observations require signed transaction, receipt inclusion, canonical finalized ancestry and exact historical state authentication.

Original signed root bytes, hash, nonce, proof and anchor are immutable. A transport timeout, absent lookup, pending competitor or source advancement cannot authorize a new signature. An authenticated external publication may satisfy the target while a local original remains unresolved: record observed progress, retain its original nonce ownership and keep that root signer fenced. An independent commitment signer may continue authenticated handovers/heartbeats. Archive proven failed originals before any later permitted publication; do not overwrite them with a competitor's success.

Unsigned stale anchors/proofs may refresh only before signing while preserving any reserved nonce. Mining alone is not finalized acceptance. Pruned-state fallback is unsupported. After interruption, resume the same identities and directory, reauthenticate retained history and reconcile original signed work before new submission. The follower's `--reconcile-once` is bounded; it is not a reset or an extended retry deadline.

Historical proof stores, backups, verifier artifacts, frozen bytecode, deployment records and already-paid historical messages remain evidence for old obligations. Removal of their producer code does not authorize deleting those stores, changing custody/maturity/replay state or rewriting a retained public actor's identity. They are not an active fallback producer.

## Inspection and incident response

Use the existing read-only commands rather than a root-proof HTTP service:

~~~sh
target/release/beefy-relay bridge-status --source-rpc "$SOURCE_RPC"
target/release/beefy-relay bridge-events --source-rpc "$SOURCE_RPC" --at-block "$SOURCE_BLOCK" --full
target/release/beefy-relay tokens-history-audit \
  --source-rpc "$SOURCE_RPC" --witness-rpc "$WITNESS_RPC" --ethereum-rpc "$EL_RPC" \
  --deployment-manifest "$DEPLOYMENT_MANIFEST" \
  --follower-state "$FOLLOWER_DIR/state.json" --proof-dir "$PRIVATE_AUDIT_DIR"
target/release/relayer fetch-merkle-roots --help
~~~

The history audit uses no signer, leaves the follower journal untouched and requires a separately owned proof directory, not the running actor's cache. Reuse that audit directory for warm recovery qualification. `fetch-merkle-roots` inspects already-published roots from `--from-eth-block` using finalized inclusion/stored-root checks; it does not generate a proof or contact a repair endpoint. Only paid delivery routes remain in the [HTTP API](usage-and-operations.md#http-management-api).

For a stuck target, inspect source block/root/snapshot availability, accepted covering anchor, registration ownership, original signed transaction/receipt, finalized destination state, maturity and journal readiness. Preserve every original before restart or endpoint change. Qualify old and new archive pins with the read-only history audit and the follower's existing bounded reconciliation mode; matching latest state is insufficient.

`relayer kill-switch` is an independent observer, not a publisher/admin repair process. It authenticates canonical destination event inclusion, including hot blocks connected to pinned finalized ancestry, checks exact stored roots at the pinned head, and compares the exact historical finalized source root. Hot blocks are revisited until finalized. Missing/pruned source state alone cannot trigger a challenge; invalid source bounds/finality or removed/inconsistent logs hold the cursor. The configured source RPC remains an operator trust boundary.

The observer retains a sent challenge hash across receipt uncertainty, then exits after a finalized successful challenge for separately authorized operator intervention. A finalized revert requires a fresh scan before another submission. It never automatically republishes or unpauses and has no admin signer or repair URL. The challenge pause is finite (currently two days); persistent incidents require the existing governance pause before expiry. Follow the [existing administration and recovery procedure](running-the-bridge.md#beefy-expiry-and-existing-administration), retaining original evidence and separate governance authority.
