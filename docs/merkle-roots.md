# Merkle roots

A Gear message-queue root commits to queued messages. Ethereum authenticates that root through the BEEFY client and a canonical MMR queue-snapshot proof, then verifies individual-message membership separately. The active outbound pipeline is **commitment follower → MMR root publisher → retained message delivery**. Updating this source does not activate a runtime, migrate public contracts/programs or approve a bootstrap.

This page describes the existing publisher, journals and recovery boundaries. See [Running the bridge](running-the-bridge.md) for env/bootstrap/profile procedures and [circuits and proofs](circuits-and-proofs.md) for proof boundaries. The independent Ethereum→Gear Beacon proof-composition path is unchanged.

## What a root represents

The Gear bridge pallet accumulates messages in a binary Merkle trie. A finalized queue-root observation has a source block number/hash, queue identity and root. Publication authenticates the historical bridge snapshot under an MMR leaf covered by an accepted BEEFY commitment. Source genesis/domain, destination chain/queue, MMR boundary, snapshot height and anchor must match the reviewed deployment.

A queue-root proof is not an individual-message proof. `MessageQueue.submitMerkleRoot(sourceBlock, root, proof)` authenticates and records the root; `MessageQueue.processMessage(...)` later checks the message's binary Merkle proof, replay protection and maturity. Publication alone is not transfer completion.

The active implementation is in [follower.rs](../tools/beefy-relay/src/follower.rs), [root_publication.rs](../tools/beefy-relay/src/root_publication.rs), [source.rs](../tools/beefy-relay/src/source.rs) and [protocol.rs](../tools/beefy-relay/src/protocol.rs). On-chain boundaries are [BeefyClient.sol](../ethereum/src/BeefyClient.sol), [VaraQueueRootVerifier.sol](../ethereum/src/VaraQueueRootVerifier.sol) and [MessageQueue.sol](../ethereum/src/MessageQueue.sol).

## Commitments, inventory and publication

`beefy-relay tokens-follow` authenticates two independent source endpoints against the original manifest/bootstrap, source binding and ordered authority history. It advances the Ethereum client using signed BEEFY commitments, retaining original transaction evidence for each submission. Authority handovers, heartbeat/client-expiry requirements and root demand remain authenticated even when root publication is held.

Run `tokens-heartbeat` as a separately supervised process to isolate client freshness from publisher HOLDs or outages. Use a dedicated approved heartbeat wallet, output directory and finality store. It needs the independently approved client/genesis/adapter/bootstrap and signer role even before binding, but no root inventory, root wallet or publication lock:

```sh
target/release/beefy-relay tokens-heartbeat \
  --source-rpc "$SOURCE_RPC" --witness-rpc "$WITNESS_RPC" \
  --ethereum-rpc "$EL_RPC" --ethereum-witness-rpc "$EL_WITNESS_RPC" \
  --wallet "$HEARTBEAT_WALLET" --deployment-manifest "$DEPLOYMENT_MANIFEST" \
  --output-dir "$HEARTBEAT_DIR" --ethereum-finality-store "$HEARTBEAT_FINALITY_STORE"
```

Also supply the approved Beacon/finality configuration described in the operating guide. The manifest's `execution.signers` binds distinct `commitment`, `root` and `heartbeat` accounts; provision the common host signer-lock namespace. No heartbeat process may reuse a follower/root journal. Forty-eight-hour publisher-outage and two-week owned-testnet qualification remain required; unit tests do not satisfy these gates.

Held, already-covered publications retry after a short maintenance window at
completed source polls or same-set commitment boundaries. This does not cancel
source work or bypass mandatory handovers and client heartbeats.

The follower inventories source progress inclusively from immutable `beefyRootMinimum`, reading paired predecessor/current `(QueueId, root)` storage in at most 32 blocks per turn without `System.Events`. New unsigned publications coalesce only after the fixed discovery target completes, selecting the newest anchor-covered snapshot per QueueId and preserving closed queues' final nonzero snapshots. An earlier discovery row becomes `covered` only after that selected publication finalizes. Queue resets never share coverage, and existing intent files/reservations remain untouched.

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

### Queue identity and cumulative delivery

Delivery journals retain `queue_id` separately from `authority_set_id`. A candidate
publication must have the same authenticated QueueId as the original message and a
source height at least as large. Its **own** finalized publication timestamp controls
maturity. GRANDPA set equality is not queue identity: stable keys can span a set
transition without resetting the queue, and overflow can reset a queue within one set.
The selected source block/hash/root and Ethereum transaction/log inclusion refer to
the actual publication, not to an earlier root supposedly registered by coverage.
Producer coalescing is not enabled by this consumer change.

Missing legacy `queue_id` is unknown, never zero. Unsigned delivery work rehydrates
it against the original canonical finalized hash, original queued event and signing
set; retained roots reauthenticate their original destination inclusion and source
snapshot. Missing archives, conflicting evidence and unknown JSON fields HOLD without
overwriting the original. Before atomic replacement, original raw JSON with missing
queue identity is fsynced under `queue-id-originals/<sha256>.json`; retain this directory
with the existing journals. Signed bytes, nonce, message identity and signed root
selection are not rewritten. A null membership RPC response is unavailable evidence,
not authenticated absence. The worker defers that exact candidate for one scheduling
cycle, allowing another qualified root (including an earlier one) and unrelated work;
the exclusion then expires. No rejection watermark or settlement is persisted, and
restart retries the original identity. Root-cache eviction is not financial completion
or proof that an older message can never be delivered.

SDK outbound preparation follows the same matching rule without reading follower
inventory `coveredBy` records. It requires historical Gear metadata and explicit
eight-byte QueueId storage at the original and candidate finalized hashes; absent
storage is not defaulted to zero. Ethereum must support `eth_call` at the retained
finalized block: preparation simulates the selected claim using the queue contract's
own maturity/governance policy. Immature or non-member candidates allow alternatives;
other execution errors stay visible. Pre-sign simulation remains mandatory. No
original signed submission, proof bytes or message identity is rewritten by discovery.

### Overflow recovery qualification

The queue cleaner uses one owned bounded retry loop and finalized, hash-pinned
`QueueOverflowedSince`/QueueId/GRANDPA observations. Storage failure is not “no
overflow.” Each attempt requests a current finalized proof whose target is at or
after overflow and whose signing set is the current set. It rechecks the tuple before
dispatch and after uncertainty; another cleaner or a set transition causes fresh
observation rather than a stale reset or process exit. RPC and submission rounds
are bounded to 90 seconds; retry backoff is capped at 60 seconds or the configured
base delay, whichever is larger.

The reconstructed VB-16 regression (supplied HTML, audited `7fe1b032`, not the
unavailable original PoC) **executes the actual runtime verifier**. Prepare an owned
loopback Gear node with an overflowing queue, then advance GRANDPA's set ID with
unchanged ordered keys and unchanged QueueId. Retain the raw pre-transition
`grandpa_proveFinality` SCALE proof covering the overflow. Independently pin the
genesis and SHA-256 of the deployed runtime `:code`; a magic-proof mock is not valid.
Provide `QUEUE_CLEANER_TEST_RPC`, `QUEUE_CLEANER_TEST_GENESIS`,
`QUEUE_CLEANER_TEST_CODE_SHA256`, funded dedicated `QUEUE_CLEANER_TEST_SURI`, and
`QUEUE_CLEANER_TEST_OLD_PROOF` (binary file path). The explicitly selected test fails
if any prerequisite is absent; it submits invalid/old proof negative controls and
the current proof, then requires finalized overflow clearance and QueueId advance:

```sh
rtk cargo test --locked --release -p relayer --lib queue_cleaner::tests::real_runtime_overflow_survives_stable_key_set_transition -- --ignored --exact --nocapture
```

Do not run this mutation test against a public network. Unit matching/membership
tests do not replace this runtime qualification.

## Durable state and uncertainty

Back up the original deployment/bootstrap records, follower `state.json`, all registered `root-publications/` intents, terminal submissions, authenticated execution proof cache and delivery discovery/transaction journals together. Use private stable paths and exclusive ownership. Do not transplant journals to another network, signer, deployment or rehearsal mode.

Follower schema 5 preserves provenance (`own` versus actual `external` accepted transactions), adds actor-role isolation and `coveredBy` references for cumulative discovery rows. Before upgrading an existing schema-3/4 state, exact original bytes are retained in `state.pre-schema5.json`. Root-intent schema 4 still distinguishes `observedProgress` from the original `publicationReceipt`. External envelopes preserve their actual nullable destination and embedded chain ID, including authenticated pre-EIP-155 legacy envelopes; no destination/chain is fabricated. Complete transaction/receipt inclusion, exact finalized effects and historical state remain mandatory.

Original signed root bytes, hash, nonce, proof and anchor are immutable. A transport timeout, absent lookup, pending competitor or source advancement cannot authorize a new signature. An authenticated external publication may satisfy the target while a local original remains unresolved: record observed progress, retain its original nonce ownership and keep that root signer fenced. An independent commitment signer may continue authenticated handovers/heartbeats. Archive proven failed originals before any later permitted publication; do not overwrite them with a competitor's success.

Missing/moved/noncanonical unfinalized inclusion is pending, not a new-signature permit. `firstInclusion` and subsequent observations remain evidence; the authoritative inclusion changes only after the same signed bytes authenticate at the new finalized inclusion. Finalized conflicts still HOLD. Authenticated finalized reverts archive the original disposition independently of an external successful effect. Known incomplete owned scratch writes are retained as non-authoritative digest-named evidence; complete conflicting scratch fences the owner.

Unsigned stale anchors/proofs may refresh only before signing **and before spend admission**, preserving any reserved nonce. A durable spend reservation also freezes its proof/anchor; retry uses the same account/nonce/immutable-proof digest and does not reserve a new daily charge. New signing requires the approved root role, paired gas/fee/balance admission and signed-envelope validation. Only an authenticated original finalized success/revert settles its spend reservation; consumed nonce without a receipt or an external successful effect does not. An unsigned external adoption may release its nonce but retains any unresolved spending reservation. Missing policy prevents new signing, not read/reconciliation. Mining alone is not finalized acceptance. Resume the same identities/directories and never reset a journal to recover budget or nonce authority.

History authentication has no aggregate startup deadline: commitment batches contain at most 32 records, and source/destination RPC bounds still apply. Root qualification and anchor caches live only for one publisher session and key exact immutable identities; restart rechecks proofs. `tokens-history-audit` authenticates all retained commitment/root history explicitly. The full follower JSON still grows with lifetime evidence.

Reconstructed regressions derive from the supplied HTML at audited revision `7fe1b032`, not the unavailable original PoC bundle. After integration, select the ordinary follower/root-publication unit tests plus:

```sh
rtk cargo test --locked --release -p beefy-relay --lib follower::tests::native_root_scan_faults_preserve_cursor -- --ignored --exact --nocapture
rtk cargo test --locked --release -p beefy-relay --lib root_publication::root_publication_tests::reconstructed_slow_history_progress_exceeds_300_seconds -- --ignored --exact --nocapture
```

The native scan regression requires paired owned archived RPCs and `SOURCE_SCAN_QUEUED_BLOCK`; it poisons full-events reads with an 8 MiB response/error while requiring storage-based progress. The slow-history test simulates per-record latency and cache/restart scheduling; it is not a real-network proof or soak result. Preserve candidate/source/artifact identities with executed results.

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
