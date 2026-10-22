# Running the bridge

This page covers the BEEFY commitment/root publisher and the retained `relayer` delivery, checkpoint and observer processes. Source changes do not activate BEEFY or migrate existing public actors; use only independently approved source, deployment and proof-profile identities.

## What you are starting

The outbound pipeline is **BEEFY commitment follower → MMR root publisher → message delivery**:

| Executable and command | Role |
| --- | --- |
| `beefy-relay tokens-follow` | Authenticates source commitments and authority handovers, advances the Ethereum BEEFY client, and publishes inventoried queue roots using a separate root signer. |
| `beefy-relay tokens-publish-root` | Publishes/reconciles an existing follower-registered root using its original publisher state and output path. |
| `beefy-relay tokens-history-audit` | Authenticates retained history read-only, without a signer. |
| `beefy-relay bridge-status` / `bridge-events` | Inspects source bridge state or events at a selected block. |
| `relayer eth-gear-core` | Reads Ethereum Beacon finality and updates the Gear checkpoint-light-client program. |
| `relayer gear-eth-tokens {all-token-transfers,paid-token-transfers}` | Delivers Gear messages after root finality and maturity. |
| `relayer eth-gear-tokens {all-token-transfers,paid-token-transfers}` | Delivers authenticated Ethereum receipts to Gear. |
| `relayer gear-eth-manual` / `eth-gear-manual` | Reconciles and delivers one known message with dedicated durable storage. |
| `relayer kill-switch` | Compares canonical destination roots, including hot blocks, with finalized source history and challenges authenticated mismatches. |
| `relayer queue-cleaner` | Performs Gear queue maintenance. |
| `relayer fetch-merkle-roots` | Inspects already-published roots using finalized destination inclusion and stored-root checks. |

The root [README](../README.md) explains the protocol-level message and token flows. The [internals](internals.md) page maps these commands to their implementation components.

## Prerequisites

For a native build, install the toolchains used by the workspace:

- Rust, using the repository's [rust-toolchain.toml](../rust-toolchain.toml).
- Approved source/archive RPCs and independently reviewed deployment/bootstrap identities.
- Foundry v1.8.6 (forge/cast, matching CI) for the Ethereum contracts and deployment tooling.
- The native build dependencies listed in [Dockerfile](../Dockerfile), including a C compiler, OpenSSL development files, CMake, protobuf compiler, and Clang.

The root README also calls out the [ring build instructions](https://github.com/gear-tech/ring/blob/main/BUILDING.md). Follow those instructions when a native build fails while compiling `ring`.

Build the relayer from the repository root:

~~~sh
cargo build --locked --release -p relayer -p beefy-relay
target/release/relayer --help
target/release/beefy-relay --help
~~~

Package-only development builds do not qualify deployment artifacts. Token deployments must use the same locked all-targets release graph described below; do not mix WASM from another feature graph with qualified deployment artifacts.

~~~sh
export RUST_LOG='relayer=info,ethereum-client=info,metrics=info'
~~~

### Native node regression tests

Run native integration cases against a fresh, owned local node matching the CI
image (`ghcr.io/gear-tech/node:v2.1.1`), with RPC on `127.0.0.1:9944`. Do not run
duplicate test processes against the same node: their deterministic accounts and
program salts can collide. Retain failures; these checks do not qualify production.

~~~sh
cargo test --locked -p tests --lib checkpoint_light_client::replay_back_and_updating -- --exact --nocapture --test-threads=1
cargo test --locked -p tests --lib relayer::eth_to_gear::test_tx_manager -- --exact --nocapture --test-threads=1
~~~

The transaction-manager test gives the serial delivery batch and the separate
wrong-genesis/paused-receipt scenario a fixed 120-second budget each. Both start
after deployment and funding; no per-transaction retry resets either budget.
These test-only bounds do not change worker or campaign deadlines.
Replay completion checks every emitted checkpoint against
the authenticated header fixture or the signed finalized target, including
checkpoints staged in the initial replay batch and committed at completion.
Replay uses 96-header batches without dropping fixture headers or increasing runtime gas limits.

## Configure outbound publication and retained workers

The executables use their existing flags, environment and durable deployment records, not a prover configuration file. Print the exact command's help before configuring it:

~~~sh
target/release/beefy-relay tokens-follow --help
target/release/beefy-relay tokens-publish-root --help
target/release/relayer gear-eth-tokens --help
target/release/relayer gear-eth-tokens paid-token-transfers --help
target/release/relayer eth-gear-core --help
~~~

Use the approved deployment manifest, original bootstrap and existing private journal directory. A continuous publisher has this shape:

~~~sh
target/release/beefy-relay tokens-follow \
  --source-rpc "$SOURCE_RPC" --witness-rpc "$WITNESS_RPC" \
  --ethereum-rpc "$EL_RPC" --ethereum-witness-rpc "$EL_WITNESS_RPC" \
  --wallet "$FOLLOWER_WALLET" --root-wallet "$ROOT_WALLET" \
  --deployment-manifest "$DEPLOYMENT_MANIFEST" --output-dir "$FOLLOWER_DIR" \
  --client-deployment-transaction "$CLIENT_DEPLOYMENT_TRANSACTION"
~~~

The two source endpoints must be independent and agree on authenticated genesis, destination binding, MMR boundary and authority history. The original client CREATE hash is an untrusted locator, not a bootstrap approval. Public Ethereum defaults to Hoodi; mainnet requires explicit `--ethereum-network mainnet` and an independently reviewed retained-state mainnet manifest. Owned fixtures require the existing explicit network approval. See [Local nodes and Hoodi](#local-nodes-and-hoodi) for the existing bootstrap/env deployment procedure and profile guards.

`tokens-follow` owns the deployment lock and inventories roots. Do not run a standalone publisher concurrently against that owner. For a reviewed interrupted publication, `tokens-publish-root` requires `--publisher-state`, `--registration` and `--output` matching the original follower inventory; it is not an arbitrary-block proof generator. See [Merkle roots](merkle-roots.md).

The retained `relayer` loads `.env` and preserves the shared `GEAR_ENDPOINT`, `GEAR_MAX_RECONNECT_ATTEMPTS`, `GEAR_SURI`, `ETH_RPC`, `ETH_BEACON_RPC`, `ETH_MESSAGE_QUEUE_ADDRESS` and command-specific signer/storage arguments. `ETH_FEE_PAYER` is the retained Ethereum delivery signer; `beefy-relay` uses protected wallet files through its own flags. Keep secrets outside the repository and storage private, persistent and dedicated to each actor.

Each Ethereum fee-payer account must have exactly one submitting process. Commitment follower, root publisher, delivery workers, manual commands and observer must use distinct configured signers; deployment and governance authorities remain separate. The in-process nonce lock cannot coordinate replicas or external tools. Do not run active-active writers on one account or share mutable journal directories.

### Independent Ethereum evidence

Every finality-bearing process requires two operationally independent Ethereum execution providers. `--ethereum-witness-rpc` / `ETH_WITNESS_RPC` is separate from the Gear `--witness-rpc`. Canonically different URLs do not prove different operators or failure domains. The approved Phase 1 `independent-two-el-v1` policy assumes **at least one provider is honest**; it is not Beacon consensus verification. Record the reviewed provider operators, failure domains and historical-data retention in the deployment approval. Both providers must serve the approved chain/genesis and required historical headers, receipts and hash-pinned state.

Production execution connections require HTTPS/WSS (HTTPS for the indexer). Plain HTTP/WS is accepted only for localhost or literal loopback addresses, reserved for owned fixtures; there is no insecure override. Endpoint comparison normalizes host case/trailing dots, default ports, trailing path slashes and loopback aliases; credentials and fragments cannot establish independence, nor can HTTP/WS or HTTPS/WSS aliases in Rust. Separate API keys, paths, queries, regions or forwarding routes still do not establish operational independence.

The common authority is the lower finalized height after agreement and parent-linked continuity, not either advertised tip alone. Missing evidence, outage, disagreement, or a common head below retained history is HOLD. Rust allows advertised-tip lag through 256 headers and rejects greater separation before common-header/ancestry requests (about 51 minutes at one block per 12 seconds). Restore healthy independent providers rather than resetting history. This is not a historical receipt-age limit. There is no single-provider/latest/safe fallback. Qualification uses bounded requests and a shared 300-second ancestry budget. Rust additionally verifies transaction and receipt MPT inclusion and exact signed originals; the indexer independently compares complete decision-relevant receipts and transaction membership, without claiming JS MPT verification.

The Rust view's immutable pin does not expire after qualification: each later historical verification has its own bounded 300-second work budget and rechecks both providers. This does not extend the BEEFY follower's separate overall startup deadline.

| Consumer | Required execution configuration and finality owner |
| --- | --- |
| `relayer gear-eth-tokens`, `gear-eth-manual`, `eth-gear-tokens`, `eth-gear-manual` | Primary connection plus witness and independently approved `--ethereum-chain-id` / `ETH_CHAIN_ID`, `--ethereum-genesis-hash` / `ETH_GENESIS_HASH`; retain `<storage_path>/ethereum-finality`. |
| `relayer kill-switch` | Same witness/approved identity; additionally require `--ethereum-finality-dir` with a dedicated persistent directory before scanning or challenging. |
| `relayer fetch-merkle-roots` | Same witness/approved identity; read-only qualification need not create an archive. |
| `beefy-relay tokens-follow`, `tokens-publish-root`, `tokens-history-audit`; `beefy-deploy tokens-manifest`, `tokens-configure` | Add `--ethereum-witness-rpc`; identity comes from the selected approved public network or owned-fixture descriptor, never endpoint discovery. Follower keeps `finality-history/headers`; history audit owns `<proof_dir>/headers`. |
| `vft-manager-tool prepare-cutover`, `finalize-cutover`, `verify-cutover` | Add `--ethereum-witness-rpc`; identity comes from retained old journal/work/seal. Prepare/finalize own `<cutover_dir>/ethereum-finality`; verify is read-only and never creates files or advances a fence. |

For retained relayer commands, set these alongside the existing command-specific source, queue, signer and storage options:

~~~sh
export ETH_WITNESS_RPC="$EL_WITNESS_RPC"
export ETH_CHAIN_ID="$APPROVED_EL_CHAIN_ID"
export ETH_GENESIS_HASH="$APPROVED_EL_GENESIS_HASH"
target/release/relayer kill-switch --ethereum-finality-dir "$OBSERVER_FINALITY_DIR"
~~~

Never share writable archives between independent owners or reset a retained fence to clear a HOLD. Reconnect preserves the witness and approval. Outbound UUID journals upgrade to version 5 only after **all** original completion/failure proofs, historical policies and nonce-disposition anchors authenticate; the durable save precedes source acknowledgements and ownership release. Existing signatures, attempts, receipts and finalized coordinates remain original evidence. An interrupted save retains the existing incomplete-write HOLD behavior.

Relayer startup still reauthenticates all retained terminal history. Provision archive retention and request capacity for that history; this phase introduces no sampling, pruning or rolling-window exception. Missing historical state remains HOLD.


## Run with Docker

The [Dockerfile](../Dockerfile) builds the complete relayer image. Build it from the repository root:

~~~sh
docker build -t gear-bridges-relayer:local -f Dockerfile .
~~~

When wrapping the image in Compose or another supervisor, mount protected keys, deployment records, authenticated history and actor journals explicitly. The image retains the `relayer` entrypoint; select `beefy-relay` or `beefy-deploy` explicitly with `--entrypoint`. Expose Prometheus and authenticated delivery HTTP ports separately and inspect the final service definition before starting it.

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

Only paid-token delivery workers expose the authenticated API: `POST /relay_messages` for Gear→Ethereum and `POST /relay_transactions` for Ethereum→Gear. Each listener enables only its own configured channel and token. Send JSON with the `X-Token` header; the body limit is 128 KiB. Success acknowledges channel handoff, not finality or settlement. See [usage and operations](usage-and-operations.md#http-management-api) for bodies and precise response handling.

Bind the authenticated API to a private interface or protect it with network policy. Prometheus is unauthenticated at the application layer, so restrict its exposure to the monitoring network.

## Shutdown, restart, and recovery

Use the process supervisor or Docker to stop the selected service cleanly. Restart with the same deployment manifest, signer identities and private durable directory; do not replace a held journal with a fresh directory.

Back up the follower `state.json`, inventoried `root-publications/`, authenticated execution-history proof cache and delivery transaction/discovery journals together with deployment/bootstrap records. The follower reauthenticates retained history and reconciles original signed transactions before fresh work. Before changing archive endpoints, use its existing `--reconcile-once` mode against the same directory and identities.

On restart, archived BEEFY scanning resumes from the accepted commitment checkpoint, not the last scanned GRANDPA block. This recovers justifications appended while the follower was offline; retained pending and delivered originals remain intact. Archive access must cover that rescan.

Retain historical proof stores, backups and old deployment material as evidence for old obligations and incident analysis. They are not active producer inputs, and this cutover does not authorize deleting them or rewriting existing public deployment records.

For a transient RPC failure, the Gear listener reconnects and replays missing finalized blocks; Ethereum pollers reconnect and resume from their persisted cursors. The shared Ethereum WebSocket handshake and individual RPC requests time out after 30 seconds for `EthApi`, `PollingEthApi` and raw-provider calls. Transient receipt-evidence transport failures reconnect and recheck the original signed transaction; identity, finality and event mismatches remain HOLD. A timeout is not proof of non-submission and does not authorize replacement signing, a different nonce or changed deadlines. A zero process exit code does not establish worker health: a failed child task can close the pipeline without a nonzero exit. Inspect logs and metrics before deciding whether to retry, restore state, or escalate to the emergency procedures.

Root qualification also replaces a disconnected Gear client and retries the same source query without advancing discovery. Semantic, identity and finality errors remain HOLD.

Wrapped HTTP transport errors are classified by typed network envelopes and their
source chains, including connection resets, unavailable local addresses after a
Wi-Fi change, unreachable/down networks, and network timeouts. Display text alone
does not make an error retryable. Explicit permanent RPC/status errors, storage
failures and ambiguous transaction-send failures keep their fail/HOLD precedence.

Both direct reqwest clients and Alloy’s separate reqwest version preserve typed
connection-establishment failures, including DNS failures without a portable
I/O kind, for backoff and reconnect. TLS validation remains enabled; retrying a
connection failure does not authorize invalid certificates or RPC identity changes.

Inbound Ethereum discovery retries the failed read without discarding its next
block or pending work. Reconnection uses exponential backoff starting at one
second, with jitter and a hard 60-second cap; downstream closure cancels the wait.

The BEEFY token follower retries transient source, witness and Ethereum transport
failures with a one-second exponential backoff capped at 30 seconds. It retains
the deployment ownership lock across retries, reloads its durable journal,
reconnects and authenticates both source endpoints, Ethereum, the bootstrap and
full retained history, then reconciles original signed transactions before
continuing. It never replaces an original transaction to recover connectivity.
`--reconcile-once` remains bounded and does not enter this retry loop. Persistence
failures, identity mismatches and the 300-second startup authentication deadline
remain terminal/HOLD conditions, not transport retries.

Accepted-commitment discovery snapshots the current Ethereum block number and
scans the complete history in explicit inclusive batches of at most 2,000 blocks.
A match does not end the scan: conflicting matches across batches HOLD, and
accepted transactions still require original signed-receipt/finality
authentication. Provider HTTP 400/range rejections remain terminal, never
transient reconnect triggers.

Laptop sleep pauses these processes; recoverable connections resume after wake
and network restoration while the processes remain alive. Process termination
or reboot still requires a supervisor. Reconnection does not bypass client
expiry, archival-evidence requirements or other safety holds.

Local Hoodi review interrupted the real execution-RPC transport for 90 seconds.
Both workers resumed with unchanged PIDs and original transaction data; inbound
discovery and follower scanning advanced after restoration. This is transport
recovery evidence, not production qualification.

The full relayer suite, six live Hoodi reconnect cases and real-contract Anvil original-receipt/restart/replay smoke passed in local review evidence. Those logs remain local evidence, not timed-campaign or production qualification.

The checkpoint worker counts consecutive Beacon fetch failures, resetting its retry budget after each successful update. Three consecutive failures close the update stream and exit the worker with an error. Submission failures, replay failures, and unexpected stream closure also exit nonzero so an on-failure supervisor can restart and reconcile from the checkpoint program's state; SIGINT remains a clean exit. Keep supervisor restart throttling enabled during a sustained RPC outage.

Every checkpoint update uses the same application path. If an already-applied committee update is followed by newer finality that requires replay, the worker replays in the same process rather than restarting into the stale committee update again. Interrupted replay keeps its original on-chain base; the local slot and processed counters advance only after replay returns `Finished`.

Inbound block snapshots serialize their writers, sync staged data, atomically replace the committed file, and sync the directory. Startup rejects interrupted snapshots or an orphaned block/transaction journal. Both journals require the same pinned Ethereum chain/genesis, deployment start block, event-manager/payment contract, Gear genesis, program identities and signer. Existing unbound history requires explicit evidence-backed reconciliation, not automatic binding to current CLI arguments. Preserve the directory when startup holds.

Outbound saves serialize transaction snapshots, sync and atomically replace individual files, and retain `.state/save.pending` until the snapshot is complete. An incomplete marker, temporary write, or malformed committed journal blocks startup; persistence failures stop processing. Preserve the entire directory for reconciliation.

`gear_events.json` uses schema 2 and binds the canonical `fee_exempt_sources` set into the outbound lane identity. Schema 1 or a missing/changed policy holds without migration. A payment-enabled lane requires the original first-fee observation unless the original message source is explicitly exempt; the default admin/pauser exemptions remain supported, including the nonce-zero bootstrap. An actually observed fee is retained even for an exempt source. Manual and all-token no-payment lanes bind an explicit empty exemption set.

Automatic completion writes an immutable `completed_nonce_<noncehex>.json` sidecar containing the original UUID, full message, source/lane and first-fee evidence. Discovery is acknowledged only after durable Completed ownership; active, held and failed work remains retained. A missing known sidecar or conflicting ownership holds during ongoing discovery as well as restart. Later normal or priority payments advance the canonical cursor without recreating completed backlog; genuinely unseen paid-first nonces remain eligible. Preserve sidecars together with their original UUID journals.

Authenticated operator requests still require the retained canonical queued observation and, where applicable, its original first-fee pair before transaction admission. An early request defers without creating a UUID or save marker. Reconnect bookkeeping deduplicates nonces through channel handoff and retires only validated durable ownership; it does not discard the original journals or genuinely unseen paid-first work.

Outbound private journal v2 separates unsigned `PrepareMessage` from signed `SendMessage`. The sender prepares locally, then waits until the manager has fsynced the original chain, contract, sender, account nonce, raw transaction, hash and proof and acknowledged that write. Only those acknowledged bytes may broadcast. Restart reconciles the original identity; a lost response or an already-finalized processed bit cannot authorize a replacement signature. Unversioned ambiguous sends, prior attempts in unsigned states, incomplete saves and changed identities HOLD with their original evidence retained.

`transaction_status.json` is the compact readiness view: version 1, an active UUID-to-status object and a failed UUID array. Startup validates it against complete journals. Readiness requires a present, consistent, empty view and no unfinished save marker; empty discovery maps alone do not prove that no payout remains. Each actor retains its authenticated Ethereum history under its own `ethereum-finality` directory.

Manual outbound relay also requires `--storage` with a private, stable directory dedicated to that one finalized source message. Reuse it after interruption; the normal durable acknowledgement and signed-identity reconciliation apply. Another message, source block, failed operation or mixed journal holds before submission. Keep one writer per signer, including manual commands. A new directory is not permission to replace an uncertain older unjournaled send; reconcile that original handoff first.

Outbound schema-4 journals migrate versions 2/3 without replacing signed bytes or prior evidence. Completion still requires the original signed transaction's canonical finalized successful receipt, configured confirmation depth, exact queue event and, for token messages, same-receipt token effect. A generic application queue with no configured ERC20 manager completes from its exact same-receipt queue evidence; this does not claim token settlement. The sender alone retries an absent original using its exact saved bytes, including after restart in WaitConfirmations and when another caller processed the message. The read-only watcher never treats that processed bit as original success. A finalized original revert is a durable failed-original outcome: retain its full receipt/finality evidence, count it once, release account ownership only after saving, and continue unrelated work without automatically signing that message again. An absent original with an authenticated finalized consumed nonce is held, not failed or completed; save and reauthenticate its nonce disposition before releasing account ownership. Neither outcome acknowledges the original source/fee event pair.

Native recovery retains every original reconciliation continuation and its authenticated terminal outcome before preparing a later continuation. Unknown replies remain HOLD; a continuation never resubmits the economic receipt or creates another payout. Reconciliation queries the original wrapper redemption and may retry wrapped credit only after its authenticated full returned backing is recorded. Completed rows are requalified at their saved finalized settlement pin, including historical manager mappings, receipt deposits and wrapper redemption state; bounded history eviction at the current head does not erase original completion evidence.

The native wrapper cannot exit, even after all payouts settle. Its constant-time outstanding count decrements only for original-child authenticated `Delivered` or fully backed `Refunded`. A refund requires returned backing **at least** the original amount, credits exactly that amount to the original Gear recipient once, and emits `ReceiptDepositRefunded`, never a native-delivery event. Excess remains wrapper-owned native backing and is retained in the evidence. Manager rotation requires paused state, **zero blocking** obligations and zero VFT minimum; held liabilities can remain financially outstanding. Existing immutable wrappers and previously reaped dust require separate liability-preserving qualification, not silent repair.

The original payout callback records full or excess returned backing without minting, whether the reply reports success or error. Until an identical `RedeemEscrow` retry or manager reconciliation successfully credits the original recipient, the obligation remains outstanding. Unheld `Queued`, `Returned` and `Ambiguous` records with backing at least the amount can retry wrapped credit; partial or unknown backing cannot. Capacity or credit failure preserves backing for retry; duplicate callbacks cannot replace it.

RPC absence, timeout, transport error or invalid finality never unlocks an unconsumed signed nonce. A permanently underpriced original can therefore fence its shared signer indefinitely; independent governance/escape signers remain the availability boundary. A retry does not allocate a nonce, sign again, create another watcher or extend an operation deadline. Manual terminal reports preserve the original hash and journal for reconciliation.

An authenticated finalized original/continuation inclusion consumes its account
nonce even while economic settlement remains pending. Such receipts retain their
original evidence and fairly reconcile without fencing unrelated fresh receipts;
unknown signed nonces and fresh continuation allocation remain serialized.

Token event listeners and paid-message requests use finalized Gear blocks without requesting a GRANDPA justification; static authority sets may not serve `grandpa_proveFinality` for such blocks. These event reads do not authenticate a queue root. Outbound publication requires the signed BEEFY commitment, authenticated snapshot and MMR proof; reverse Beacon proof composition and GRANDPA-dependent discovery/queue-cleaner flows remain independent.

Keep each deployment's token-relayer journals under a dedicated storage path. The Ethereum-to-Gear receipt journal uses schema 5; an older journal fails to load instead of silently replaying a transfer. Preserve old journals for separate reconciliation; an isolated fresh deployment uses a new storage path and does not depend on repairing the retiring lane. Never relabel or transplant old journals. A held receipt (including an Unknown or unavailable finalized status) is not permission to submit it again; inspect the original payload and finalized VFT manager status. The journal retains the original receipt key, proof payload, composition/handoff times, and first submission reply or error with its timestamp and sender/program identities. Later finalized-status observations and query diagnostics cannot overwrite that evidence, including after completion or restart. Gear-to-Ethereum queued/paid cursors and observations must stay with its journal across restarts.

A schema-5 unsigned `SubmitMessage` with no handoff, or `PreparingSubmission`, resumes its original fully decoded receipt proof; a queued slot/index mismatch or trailing SCALE data holds before signing. Ambiguous unsigned handoffs remain status-only. Historical finalized-no-effect attempts remain archived when a genuinely retry-ready current attempt resumes; they are not confused with a new ambiguous handoff. Schema 4 and earlier do not establish the original-transaction binding guarantee and remain HOLD without rewriting their UUIDs, hashes or proof bytes.

Sender responses wake the shared inbound transaction manager instead of waiting for its 12-second idle reconciliation timer. Only prepared signatures and successful completions immediately advance queued work; unresolved responses do not immediately requeue themselves. Signing remains serialized, and prepared original bytes are still persisted before submission; a response does not authorize a replacement signature or skip finalized reconciliation.

New inbound attempts persist the exact signed Gear extrinsic, hash, account nonce, chain/program identities, receipt binding, and finalized scan checkpoints before broadcast. Restart reconciles or rebroadcasts those same signed bytes; it does not sign a replacement merely because receipt status is Unknown. A new attempt requires finalized no-effect evidence: a failed dispatch before message queuing, a complete canonical scan proving the original extrinsic absent through a finalized consumed nonce, or a retryable per-log VFT reply. Preserve every prior attempt. In a compatible schema-5 journal, a held entry without signed identity remains status-query-only; unsupported older schemas never enter the worker.

Even when the manager reports a receipt already processed, the worker first checks the persisted signed attempt against the current Gear genesis, sender, manager/proxy, receipt payload, and exact transaction hash. A coincident receipt key on another deployment cannot complete a mismatched journal. Before returning a proof, composition authenticates the full Beacon block against its checkpoint chain, binds the selected raw transaction hash and index, and verifies receipt metadata and the MPT value against the authenticated receipts root. A Processed shortcut cannot replace these checks.

Manual inbound relay requires a private, stable `--storage` directory with exclusive ownership. Reruns bind the same original Ethereum transaction/inclusion, receipt slot/index, route, deployment and Gear signer, and restore the shared durable sender before work starts. Its optional schema-5 `manual_identity` cannot be adopted by automatic workers. Changed identities or signed bytes (including completed and archived attempts) hold without rewriting evidence; successful completed manager reruns require historical and current canonical Processed observations. Custom consumers instead require the exact-route Sails SCALE `Result<T,E>` from the authenticated original finalized reply. Their opaque `Ok` body proves application acceptance, not token settlement; `Err`, malformed envelopes and route mismatches remain unresolved. Never clear held storage to obtain a new signature. Before restoring any transaction, every persisted map key must equal its embedded UUID, and active/completed journals must not share a UUID; conflicting ownership holds without collapsing signed attempts into another transaction.

The sender reports a post-dequeue balance-RPC failure through the original request UUID rather than losing the consumed request. Run the retained live-RPC regression with `GEAR_BALANCE_FAULT_RPC=<read-only fault proxy> cargo test --locked -p relayer --lib message_relayer::eth_to_gear::message_sender::tests::post_dequeue_balance_fault_reports_original_request -- --ignored --exact --test-threads=1`. The proxy must pass the first `System.Account` storage read, fail the second with `balance-fault-after-dequeue`, and reject mutating RPC methods. This check prepares no signed output and broadcasts nothing.

The VFT manager accounts for every matching deposit log in an authenticated receipt, retaining per-log progress across retryable partial failures. Completed logs are not credited again; ambiguous mint/unlock effects remain held. Existing whole-receipt processed and reserved keys still block replay, so this change does not automatically backfill previously stranded logs in a legacy processed receipt.

The deployable full Cargo graph can enable VFT benchmark routes. All three benchmark mutations require the existing admin before sending messages or changing receipt history or token mappings. Qualify their authorization and accounting against that same release-profile WASM, not a feature-reduced replacement.

Inbound token workers restore receipt and block journals before starting listeners; unreadable journals stop startup. Backfill runs oldest-first and retains a block until processing succeeds, so another disconnect cannot advance the cursor past unfinished work. Restart with the same storage path rather than clearing it.

Inbound token workers require `--ethereum-blocks <discovery.json>` and `--ethereum-start-block <deployment-block>`. Discovery schema 1 pins the complete runtime identity, original deployment start, contiguous discovered/extracted block-hash cursors and pending transaction handoffs. Keep it paired with transaction state and separate from the transaction directory’s `blocks.json`. Restart authenticates cursor/pending coverage and replays only pending work before scanning new blocks; bounded channel backpressure replaces all-history height replay. Each handoff is durable before extraction is acknowledged and is cleared only after transaction intent is durable. Unbound legacy arrays, a changed start/identity, gaps, missing original hashes, incomplete journal pairs or interrupted snapshots HOLD unchanged; never bump the start to bypass them. Completed proof evidence remains retained: real transitions still persist it, but idle ticks no longer rewrite the full history.

The BEEFY publisher persists its transaction intent, source block identity, and nonce before broadcast; after an uncertain send it reconciles the same transaction and canonical contract state instead of choosing a new nonce. Fresh commitment records include the source block number before the first nonce-reservation checkpoint. Restart selects that exact reserved source block rather than an earlier eligible handover; an archive that passes it without finding it holds the lane. Saved signed bytes are decoded completely and checked against their hash, sender, nonce, and destination. RPC transaction lookups decode only the required identity and optional inclusion fields, so a missing `accessList` does not prevent recovery; malformed or conflicting identity still holds the operation. Canonical receipt and checkpoint verification remain mandatory. Keep root publication and token relayer signing keys separate. A conflicting root or source reorganization requires operator investigation; do not discard the journal to force progress.

Follower schema 4 separates commitment provenance: `origin: "own"` retains `submission`; `origin: "external"` retains the actual accepted signed `acceptedTransaction`. First attachment may adopt progress beyond the authenticated deployment bootstrap, but only after ordered source authority handovers and canonical finalized transaction/receipt proofs are verified. A log or current checkpoint alone is insufficient. `lastSuccessfulUpdate` remains own-success accounting; authenticated external progress may advance finalized/observed progress without completing an original transaction. Signed originals retain nonce ownership until their own finalized receipt or consumed-nonce disposition is durable; terminal originals move atomically into `terminalSubmissions` with complete original bytes and evidence. Competitor hashes never replace originals. Changed unfinalized inclusion requires independent finalized-header, transaction/receipt-trie, source-proof and accepted-checkpoint authentication; `firstInclusion` remains retained and finalized history cannot move. Idle same-set commitments are skipped; authenticated handovers, uncovered root demand and the 12-hour heartbeat drive paid updates. Readiness uses authenticated source age and the onchain 24-hour expiry, not an obsolete 64-block lag policy.

On follower restart, saved finalized commitments are revalidated against their original signed identity, receipt inclusion, finalized anchor and source block before any new submission. A finalized boolean is not sufficient. The worker uses one fresh verified finalized view, deduplicated targets and bounded 32-way checks. Durable hash-linked RLP headers and transaction/receipt trie evidence are locally authenticated on reload; incomplete ancestry fences and interior progress survive interruption beyond the unchanged 32,768-header memory bound. History revalidation and pending-transaction reconciliation share one 300-second startup deadline. Changed or incomplete evidence holds instead of skipping history, resetting the journal or extending warmup deadlines.

BEEFY recovery verifies embedded original receipt proofs against freshly qualified inclusion/finality headers, without requiring a separate receipt-cache file or historical block bodies for those proofs. Missing legacy proof fields still require cache/RPC reconstruction; malformed or contradictory retained proofs HOLD instead of falling back. Historical contract-state and source-chain checks remain required.

`beefy-relay tokens-history-audit --source-rpc <source> --witness-rpc <Gear-witness> --ethereum-rpc <EL> --ethereum-witness-rpc <independent-EL-witness> --deployment-manifest <manifest> --follower-state <existing-state.json> --proof-dir <new-private-audit-directory>` exercises that read-only historical authentication path without a signer. Keep the original journal untouched and use a separately owned proof directory, never the running follower’s archive. Repeat with the same audit directory for the warm-recovery check; fresh deployment history is not evidence for aged-journal recovery.

Historical endpoint qualification (not a promise of current availability):

The Hoodi CLI default is `https://hoodi.drpc.org`. Its hash-pinned historical state and canonical receipts were checked against the retained v2.1.0 deployment; the read-only history audit authenticated all 693 originals in 58.1 seconds cold and 2.0 seconds warm without changing the follower journal. This qualifies that observed history, not a provider SLA or a mainnet endpoint. The previously used ethpandaops endpoint returned malformed null contract results/timeouts; PublicNode could not serve required old state. Do not substitute latest state, clear journals, fabricate missing results, or automatically rebroadcast through an unqualified provider. Before an endpoint switch, verify the approved chain/genesis, oldest and newest retained EIP-1898 state pins, original transaction/receipt identities, and both cold/warm history audits within the existing startup deadline. Then restart the single existing journal owner with an explicit `--ethereum-rpc`, unchanged wallets/deployment/output directory, and inspect canonical progress. Budget a dedicated archival execution service and historical Beacon access for production; public free access alone is not a production availability commitment.

During subsequent PR finalization, dRPC rejected explicit 500–2,000-block log
ranges with HTTP 400, despite accepting 100-block ranges; PublicNode again lacked
required historical state. The existing `https://rpc.hoodi.ethpandaops.io` endpoint
was requalified with the original journal and full startup authentication. The
follower resumed there, preserved prior signed transactions and accepted the
original root for Gear block 48194. The corresponding 100-token user transfer
completed through its original canonical finalized receipt, with the expected
finalized recipient balance. These observations supersede the earlier endpoint
availability result, not the archival checks or production qualification gates.

Live recovery preserved every prior signed submission and authenticated the two
catch-up handovers through finalized original receipts, reaching 695 finalized
records without replacing a nonce or signature. The existing outbound worker
was rebuilt and restarted against the same archive, wallet and durable storage;
its completed transfer record remained unchanged and HTTP root polling resumed.
Sparse scheduling still promotes receipt finality on its existing freshness
check interval. After an authenticated near-head same-set scan, an idle restart
becomes healthy without buying a transaction or extending the accepted expiry.

Back up the authenticated execution proof cache with the signed journals. The
693-record timing is not a guarantee for an arbitrarily old cold journal; use
the read-only audit to warm/qualify retained history before moving an aged actor,
and retain resumable evidence rather than weakening the startup deadline.


Public Ethereum selection is explicit: `--ethereum-network mainnet` applies to the follower, root publisher and history audit; the default remains Hoodi. Mainnet requires an independently reviewed `mainnet-token-stack` deployment with chain 1, no rehearsal/fixture approval, and HTTPS or WSS. Its execution genesis and finalized-history archive are pinned to Ethereum mainnet, never learned as approval from the endpoint. Existing Hoodi journals retain their original modes and identities; do not relabel them for mainnet. Fresh `tokens-manifest`/`tokens-configure` tooling remains Hoodi/owned-fixture scoped; mainnet consumes the separately reviewed retained-state migration manifest.

The read-only execution-boundary check is `BEEFY_TEST_MAINNET_RPC=<mainnet-EL> cargo test --locked -p beefy-relay --lib ethereum::tests::qualify_mainnet_execution_finality_read_only -- --ignored --exact --nocapture`. It verifies chain/genesis isolation and canonical finalized execution-header identity without a supplied signer or transaction. It does not qualify Beacon proofs, a production BEEFY bootstrap, retained liabilities, governance or an end-to-end mainnet deployment.

Root-intent schema 4 distinguishes `observed` external progress from an original `publicationReceipt`. Matching unsigned roots and unsigned empty targets already covered by finalized monotonic progress need no broadcast, but require authenticated signed transaction, receipt and inclusion/finality evidence in `observedProgress`. Unfinalized competitors remain pending; conflicting roots HOLD. Refresh a stale anchor/proof only before signing, preserving any reserved nonce. After signing, anchor transaction/checkpoint, proof, queue proof, nonce, hash and bytes remain immutable. A proven failed original is archived before a later publication may be prepared. Canonical mining is not final acceptance. Use an archival execution RPC; pruned-state fallback is unsupported.

Authenticated external adoption releases an unsigned root intent's nonce reservation, including a stale reservation recovered on restart. This applies only when no local signed transaction exists; signed or ambiguous originals remain fenced.

When a signed root original remains unresolved but authenticated `observedProgress` satisfies its target, the root stays `pending` and retains its exact signature, nonce and anchor. Reconciliation returns promptly instead of waiting out the original-receipt timeout on every follower tick; the independent BEEFY signer may advance. The root signer remains fenced, and a later original revert is archived as a failure, never replaced by the competitor's success. Missing, conflicting or reorged external evidence still holds. The standalone `tokens-publish-root` command finishes on authenticated `Observed` as well as original `Accepted` and releases its deployment lock; this does not report an unresolved local original as successful.

The ignored `follower::tests::authenticated_competing_root_unfences_only_independent_follower` regression requires `BEEFY_TEST_ROOT_FORK_RPC` pointing to an explicitly owned loopback Anvil fork of Hoodi block 3810598 and `BEEFY_TEST_ROOT_FORK_DIR` containing public-evidence copies named `deployment.json`, `follower-original.json`, and `published-root.json` (root 3708). It authenticates the genuine external publication, exercises the actual follower Pending transition, and replays the next already-public signed commitment only locally before reconciling a distinct failed root original. It is a source-independent EVM actor/barrier check, not fresh native traffic, a standalone publisher qualification, or full-stack settlement. Keep the output directory new; never target a live endpoint or original journal.

Historical source inventory starts at immutable `beefyRootMinimum`, not first publication or maximum progress. Later roots or empty progress do not satisfy an older missing nonzero epoch. Bounded-distance catch-up requires matching initialized source/witness snapshots and publishes their actual root: zero uses empty progress; nonzero uses ordinary root publication. Every newly backfilled block has its own maturity delay.

Outbound discovery has one atomic `ethereum_roots.json` owner. It binds retained roots to finalized Ethereum block hashes and transaction/log coordinates; subscriptions only wake finalized catch-up. The extractor saves roots before in-memory acceptance and advances its hash-bound cursor only after all acknowledgements. A legacy numeric cursor/cache triggers a full replay from block zero in 2,000-block batches to an authenticated saved target, never a fixed rewind. Old files remain evidence, not runtime inputs. Rebuilding roots cannot reach the accumulator or unsigned sender; restart resumes and revalidates the rebuild before hydrating ready state. Preserve historical execution headers and source archive state. Authenticated conflicts HOLD; unavailable finality retries without cursor advance. Saved unsigned selections are requalified before preparation; original signatures are never rewritten.

The root cache retains at most 100 authenticated publications globally, by Ethereum publication order rather than source height. A newly published historical root can therefore enter a full cache without displacing its own proof merely because later source heights were published first.

### Monthly operating-cost envelope

The gross target is **under US$5,000 per 30 days before fee income**. Bridged
principal is not operating treasury. The [receipt benchmark](../ethereum/README.md#full-transaction-beefy-gas-and-operating-cost)
includes Ethereum intrinsic/calldata costs; it is not a production gas ceiling.

The follower no longer buys an idle update every 32 source blocks. It preserves
ordered authority handovers, a 12-hour heartbeat, 24-hour expiry and root demand.
At the observed native 3-second/2,400-block epoch cadence, idle updates fall from
about 900/day to 12/day (98.7% fewer); verify the activated mainnet schedule.
New unsigned token executions use the existing Alloy fee-history estimator,
not a fixed 0.5-gwei minimum tip. The former 2-gwei max-fee floor was a cap,
not the amount necessarily paid. Original signed transactions are never repriced.
The local signing/mining regression is
`cargo test --locked -p ethereum-client --lib tests::content_message_follows_market_fees_without_fixed_tip -- --ignored --exact --nocapture`.

Planning assumptions, not exchange-rate/vendor quotes: ETH = $3,000, VARA =
$0.001, 59 signing validators, 360 handovers/month. The finalized mainnet roster
read at block 36,913,322 contained 59 validators and runtime spec 11000; that
observation does not certify the pending BEEFY activation. Reprice after the
runtime upgrade, a roster change or a source-schedule change.

| Monthly allocation | USD | Qualification |
|---|---:|---|
| Two independent Vara archive nodes | 500 | Procurement allowance; production archive growth/sizing not yet measured |
| Archival Ethereum execution + historical Beacon RPC | 250 | Allowance; require explicit history retention, quotas and availability terms |
| Relayer/indexer hosting | 150 | Allowance; no recurring ZK prover/GPU assumed for this BEEFY lane |
| Backups and monitoring | 100 | Allowance; preserve signed journals and authenticated proof cache |
| Vara execution/checkpoint costs | 100 | Local checkpoint estimate about 43,000 VARA/month plus about 3.96 VARA/inbound; covers 100 inbound/day at the planning rate, not production weights |
| Operational contingency | 500 | Failed originals, exceptional maintenance and variance; not bridged funds |

Use gas allowances of 500,000/handover, 400,000/additional demand update,
200,000/root publication and 180,000/ERC20 execution. These are margins over
measured cases, not universal maxima. Charge each outbound transfer its own
additional update, root and execution (780,000 gas), with no batching savings.
Allow 15 extra empty-progress root publications/month; measure actual non-message
root events/progress and charge any excess as well. Include deployment/migration
separately: these are steady-state operating scenarios, not the launch budget.

The previously measured fresh 16-transaction Hoodi deployment used 18,721,390
gas (about $56.16 at the planning 1-gwei/$3,000 assumptions); that is not a
retained-state mainnet migration quote. Charge actual launch transactions to
the first-month budget/contingency rather than treating them as free.
User-facing service-fee schedules are unchanged by these operating-cost fixes.

| Outbound ERC20 volume | Effective gas price, including tip | Ethereum USD/month | Total USD/month, including all allocations above |
|---|---:|---:|---:|
| 35/day | 1 gwei | 3,006.00 | 4,606.00 |
| 100/day | 0.4 gwei | 3,027.60 | 4,627.60 |
| 100/day | 1 gwei | 7,569.00 | 9,169.00 |

Recommended launch planning envelope: 35 outbound/day at a 1-gwei monthly
weighted effective price, or 100/day only while the weighted price is at most
0.4 gwei. These are capacity/budget conditions, **not implemented admission
limits or price guarantees**. Native/custom token paths need their own measured
allowance. Reforecast receipts, volume, roster, exchange rates and provider
invoices daily. At roughly 6.2 gwei the mandatory handover baseline alone exhausts
the available Ethereum allowance; this architecture cannot guarantee $5,000 at
arbitrary congestion even with zero users. Never enforce the budget by skipping
handovers, weakening proofs, starving an already signed original or using
custody balances. Escalate funding/traffic/SLA decisions before the envelope is
exceeded; existing liabilities and messages must remain recoverable.

The next cost design is unsigned **root coalescing within each QueueId**, not
merely delaying the existing publisher: the runtime's cumulative root changes
once per changed source block, and the current publisher retains each row.
The implemented frozen demand batch shares a BEEFY update, not publications.
A later root may cover earlier messages in the same queue epoch, but a cutover
must prove complete nonce/leaf coverage, preserve every nonzero queue epoch and
historical claim, update proof/discovery consumers together, and leave signed
originals immutable. Qualify overflow, epoch reset, backfill and restart before
root elimination. Aligning a valid final root with each 2-hour handover could
avoid extra demand updates, at up to roughly 2 hours of additional batching
latency. With exactly one root per handover, 100 ERC20 executions/day and the
same allowances, the indicative 1-gwei total is $3,985/month; queue overflow,
extra epochs and stricter latency increase it. This is a researched design,
**not deployed savings**. Do not change quorum/sample security or migrate custody
to another chain to make the estimate fit.

Free public dRPC currently resolves the observed Hoodi archive failure, but is
not the production availability plan. [dRPC pricing](https://drpc.org/pricing)
and [request tiers](https://drpc.org/docs/pricing/requests) distinguish public
free capacity from paid service; obtain the actual archive/Beacon coverage and
price before procurement. [Vara node requirements](https://wiki.vara.network/docs/vara-network/node)
are node minima, not archive disk forecasts. Reuse the two existing local archive
nodes for qualification; provision production history deliberately rather than
assuming a pruned RPC can recover old state. Provider allowances above are not
purchases or claimed SLAs.
For a concrete procurement candidate, [Chainstack advertises Ethereum archive
access from $49/month](https://chainstack.com/build-better-with-ethereum) and
documents a [Beacon API](https://docs.chainstack.com/reference/beacon-chain).
Those pages do not establish the historical Beacon-state retention needed here;
verify it and replay the pinned EL/Beacon history before approving a plan. No
paid provider credentials were present in the existing follower configuration.

### BEEFY expiry and existing administration

[MessageQueue](../ethereum/src/MessageQueue.sol) retains its UUPS `DEFAULT_ADMIN_ROLE`, initially held by [GovernanceAdmin](../ethereum/src/GovernanceAdmin.sol). The admin-only `migrateToBeefy` uses `reinitializer(8)` and grants no UUPS roles. Its fifth argument separately binds the approved recovery timelock; expected-verifier/maximum guards and the paused cutover preserve custody, roots, maturity, genesis and replay state.

See the [mainnet migration candidate](../ethereum/README.md#mainnet-custody-preserving-migration-candidate) for retained-state fork evidence and the source activation, mainnet follower, full-accounting and governance gates. Authority impersonation and fixture BEEFY checkpoints are local test mechanisms, not production authorization.

A BEEFY client expires after 24 hours without an authenticated source timestamp. Source-only governance cannot repair an expired proof lane. The independently callable positive-delay timelock may execute the bounded `recoverBeefyVerifier` route, preserving source/MMR identity and requiring a live forward checkpoint. Recovery leaves the queue paused; a new authenticated source-governance message may unpause it. Review the [controller policy](../ethereum/README.md#expiry-recovery-and-initializer-scope), including actual multisig/role configuration, before approval; relay signers receive no recovery authority.

Fresh deployments use `initializeBeefy` with governance admin, governance pauser, emergency-stop admin/observers, verifier and approved recovery timelock. It validates the actual proxy binding and pins the BEEFY floor; it does not repeat immutable constructor policy probes. It is not an initializer for an already initialized ZK proxy. Native deployments retain Foundry broadcasts under their run's `foundry-broadcast/` directory. Do not reset retained campaign clocks, failed verdicts or unresolved obligations.

Operational root-publication holds do not stop independent authenticated BEEFY
heartbeats/handovers. Publisher nonce ownership covers every inventoried journal,
not just the selected root. Unknown originals remain fenced; client advancement
may stale their proofs, but only exact-byte settlement and authenticated finalized
no-effect evidence permit fresh proof/signing. Source/proof conflicts remain fatal.

### Observer challenge and operator response

`relayer kill-switch` requires the execution witness/approved identity and a dedicated `--ethereum-finality-dir`, alongside the existing connection/metrics arguments, optional `--from-eth-block`, and `--eth-observer-pk-path` (`ETH_OBSERVER_PK_PATH`). Supply only the separately funded observer-role key, never an admin or publisher key. It has no repair URL or HTTP dependency.

The observer pins the latest destination head and authenticates its ancestry back to finalized history, each event's inclusion, and the exact stored root at that head. It compares the exact historical finalized source root. Its cursor advances only through finalized destination blocks, so hot blocks and replacements after a reorg are revisited. Missing/pruned source state alone cannot trigger a challenge; invalid source bounds/finality, removed logs and inconsistent destination evidence hold the cursor. An already authenticated mismatch is not suppressed by later unavailable source state. The configured source RPC remains an operator trust boundary.

The pinned head is rechecked for canonicality before acting on a mismatch or advancing the cursor. Scanning hot blocks avoids waiting for Ethereum finality beyond the five-minute user/pauser maturity window. A challenge submitted before finality can outlive a later reorg; reconcile its original evidence before separately authorizing challenge removal or unpause.

A sent challenge hash is retained across receipt retries; uncertainty is not permission to send again. A canonical finalized revert requires a fresh scan, not a blind retry against the old hot root. A finalized successful challenge exits for separately authorized operator intervention, without automatic republishing or unpausing. The challenge pause is finite (`CHALLENGE_ROOT_DELAY`, currently two days); a persistent incident requires the existing governance pause before expiry. Preserve original evidence and use the existing reviewed governance/recovery procedure with separate authorities, not a restarted watchdog as a repair loop.

## Local nodes and Hoodi

This is a fresh isolated deployment, not a public ZK-proxy migration. Use an approved BEEFY-capable Gear binary/runtime, two local archive validators, funded test accounts, and Hoodi execution/Beacon endpoints. Keep existing nodes, keystores, signed journals and unresolved transfers intact. A fast runtime can shorten functional feedback, but its results do not establish normal-cadence behavior.

Build the same locked release graph used for deployment and CI:

~~~sh
cargo build --locked --release --all-targets --features beefy-relay/deployment
~~~

The normal `beefy-relay` actor has no deployment or campaign dependencies. `beefy-deploy` is the opt-in local bootstrap/token tool. Retain each Gear program's lockfile: nested WASM builds use it independently of the workspace lock. Authenticate the actual optimized CodeIds; rebuilding a different feature graph is not an identity-preserving replacement.

### 1. Fresh source identity and nodes

Set `GEAR` to the approved node binary, `RUN` to a new private directory, and `DEPLOYER`/`HOODI_HTTP_RPC` to the funded Ethereum deployment account and endpoint. Reserve that account's nonce until deployment; do not use it for another process. The token profile's queue offset is defined beside deployment order in [Base.sol](../ethereum/test/Base.sol).

~~~sh
umask 077
mkdir "$RUN"
EXPECTED_DEPLOYER_NONCE=$(cast nonce "$DEPLOYER" --rpc-url "$HOODI_HTTP_RPC")
QUEUE=$(cast compute-address "$DEPLOYER" --nonce "$((EXPECTED_DEPLOYER_NONCE + 12))" | cut -d' ' -f3)
"$GEAR" build-spec --chain local --raw --disable-default-bootnode > "$RUN/chain.raw.json"
mkdir "$RUN/alice" "$RUN/bob"
"$GEAR" key generate-node-key --chain dev --file "$RUN/alice/network-key"
"$GEAR" key generate-node-key --chain dev --file "$RUN/bob/network-key"
ALICE_PEER=$("$GEAR" key inspect-node-key --file "$RUN/alice/network-key")
~~~

Verify the embedded runtime `:code` against the chosen v2.1.0 artifact. Use the unmodified local preset: do not inject a domain or BEEFY genesis. Its development runtime is not a production mainnet Wasm. Keep normal epoch/slot timing and the funded Alice/Bob validator entries. Existing-chain upgrades follow the [governed procedure](runtime-upgrade-and-beefy-activation.md).

Run each node in its own terminal or existing service manager. RPC stays on loopback; do not add `--rpc-external`.

~~~sh
"$GEAR" --chain "$RUN/chain.raw.json" --base-path "$RUN/alice" --alice --validator \
  --force-authoring --node-key-file "$RUN/alice/network-key" \
  --listen-addr /ip4/127.0.0.1/tcp/30334 --public-addr /ip4/127.0.0.1/tcp/30334 \
  --rpc-port 9944 --rpc-methods unsafe --prometheus-port 9615 \
  --state-pruning archive --blocks-pruning archive --enable-offchain-indexing=true \
  --offchain-worker always --no-mdns --no-telemetry
~~~

~~~sh
"$GEAR" --chain "$RUN/chain.raw.json" --base-path "$RUN/bob" --bob --validator \
  --force-authoring --node-key-file "$RUN/bob/network-key" \
  --listen-addr /ip4/127.0.0.1/tcp/30335 --public-addr /ip4/127.0.0.1/tcp/30335 \
  --bootnodes "/ip4/127.0.0.1/tcp/30334/p2p/$ALICE_PEER" \
  --rpc-port 9945 --rpc-methods unsafe --prometheus-port 9616 \
  --state-pruning archive --blocks-pruning archive --enable-offchain-indexing=true \
  --offchain-worker always --no-mdns --no-telemetry
~~~

After both nodes finalize, require the same `chain_getBlockHash(0)` and runtime code/metadata; record `SOURCE_GENESIS`. Set `SOURCE_RPC=ws://127.0.0.1:9944`, `WITNESS_RPC=ws://127.0.0.1:9945`, and **`SOURCE_DOMAIN=$SOURCE_GENESIS`**. Under the native protocol these are the same identity, not independently generated values. Compute the domain from that genesis, Hoodi chain ID and the reserved queue prediction:

~~~sh
SOURCE_DOMAIN="$SOURCE_GENESIS"
BEEFY_BRIDGE_DOMAIN=$(cast keccak "$(cast abi-encode --packed 'f(string,bytes32,uint256,address)' \
  'vara/gear-eth-bridge-domain/v2' "$SOURCE_DOMAIN" 560048 "$QUEUE")")
~~~

Fund each actual session-key owner with sufficient **unbonded** fee balance first. Generate/register native owner-bound keys on the correct validator node; both loopback endpoints must agree on genesis, finalized history and SessionKeys API v2. The helper rotates all five keys using `author_rotateKeysWithOwner`, including its native ownership proof; no separate BEEFY SURI or empty proof is accepted. Keep old private keystore entries for current/queued sessions:

~~~sh
target/release/beefy-deploy source-keys \
  --source-rpc "$SOURCE_RPC" --witness-rpc "$WITNESS_RPC" \
  --expected-genesis "$SOURCE_GENESIS" --authority alice --output-dir "$RUN/source-keys-alice"
target/release/beefy-deploy source-keys \
  --source-rpc "$WITNESS_RPC" --witness-rpc "$SOURCE_RPC" \
  --expected-genesis "$SOURCE_GENESIS" --authority bob --output-dir "$RUN/source-keys-bob"
target/release/beefy-relay bridge-status --source-rpc "$SOURCE_RPC"
target/release/beefy-relay bridge-status --source-rpc "$WITNESS_RPC"
~~~

Each native registration saves its original signed bytes before submission; resume the same authority/directory without generating replacement keys or signatures. New local-helper originals are explicitly immortal; retain any older failed/expired mortal record separately. A finalized `NextKeys` write does not activate those keys.

Through the authorized local Sudo account and current metadata, bind the paused source with `gearEthBridge.bindDestination(chainId32BE, QUEUE)`, then separately schedule first activation with `beefy.setNewGenesis(positiveDelay)` while `Beefy.GenesisBlock` is absent. Follow the [native preconditions and weight rules](runtime-upgrade-and-beefy-activation.md#6-bind-the-destination-and-activate-through-native-calls): mixed Normal/Operational batching can exceed the Normal limit. Do not use raw storage, an unchecked weight, a reset or a custom genesis shortcut. Verify the finalized native tuple/domain and actual activation block on both nodes.

Wait for observed session/key transitions until both bridge statuses report `initialized=true`; a fresh standard local chain can need two two-hour session transitions for its first real GRANDPA key change. `bridge-initialize` waits for that state and unpauses; it does **not** bind a destination, activate BEEFY or bypass native initialization. Then:

~~~sh
target/release/beefy-deploy bridge-initialize \
  --source-rpc "$SOURCE_RPC" --expected-genesis "$SOURCE_GENESIS"
target/release/beefy-deploy tokens-anchor \
  --source-rpc "$SOURCE_RPC" --witness-rpc "$WITNESS_RPC" > "$RUN/anchor.json"
~~~

Require initialized/unpaused status on both nodes before traffic. A signed bootstrap may be prepared earlier only after native binding/activation and genuine signed post-binding commitments exist; keep traffic held until initialization and all profile gates pass. Check its actual genesis/domain, runtime identity, independent first-MMR boundary, activation and current/next sets. Canonical signatures/proofs are required; matching RPC values alone are not independent trust.

### 2. Checkpoint, programs and Ethereum contracts

The existing **release `checkpoints-tool`** uploads and initializes checkpoint-light-client directly; no separate `eth-light-client` executable is required. Use authenticated optimized release Wasm: the debug helper embeds debug Wasm and exhausted native gas on the real 512-key Hoodi committee. Do not raise runtime gas limits or weaken crypto validation to accommodate that artifact. Supply an independently trusted Hoodi weak-subjectivity root and genesis validators root through `--trusted-bootstrap-root` and `--trusted-genesis-validators-root`; the endpoint supplies data, not trust. Select a bootstrap compatible with an available signed update. Use the resulting **applied checkpoint slot/root**, not merely the bootstrap header, for token setup. The program verifies committee proofs and signed updates.

Keep the checkpoint updater on a distinct funded local signer. Choose a batch size that fits the native block budget (`--size-batch-multiplier 1` gives 32 headers, rather than the default 960, in the exercised updater). An RPC interruption or terminal program error requires original-result and applied-progress reconciliation before resuming the same checkpoint; never create a replacement checkpoint as a retry.

Keep a protected `GEAR_SURI_FILE` for the local setup account. It must be distinct from relayer accounts. With `CHECKPOINT_PROGRAM`, `CHECKPOINT_SLOT` and `CHECKPOINT_ROOT` set to that authenticated deployment:

~~~sh
target/release/beefy-deploy tokens --source-rpc "$SOURCE_RPC" \
  --expected-genesis "$SOURCE_GENESIS" --gear-suri-file "$GEAR_SURI_FILE" \
  --checkpoint "$CHECKPOINT_PROGRAM" --checkpoint-slot "$CHECKPOINT_SLOT" \
  --checkpoint-hash "$CHECKPOINT_ROOT" --output-dir "$RUN/tokens"
~~~

The durable output is `$RUN/tokens/token-stack.json`. Preparation allocates the configured balance/allowance shards and keeps the manager paused. Resume the original state rather than reconstructing programs. Four Ethereum-supply assets (USDC, USDT, WBTC, WETH), Gear-origin GOT and native WTVARA are configured; native value must be backed by actual deposits, never administrative minting.

Preparation and configuration hold the canonical `token-stack.lock` for the whole operation; configuration acquires the deployment lock first. New constructors start unsigned. Every Init/Handle call persists its original raw extrinsic, hash, nonce, signer, immutable call intent and canonical scan anchor before broadcast. Restart scans authenticated finalized originals and replays their exact replies; an absent original may rebroadcast only its saved bytes after a complete archive scan and unconsumed nonce check. The original broadcast watch is retained through finality rather than repeatedly submitting a still-included transaction. Recovery decodes metadata-checked `MessageQueued` and `UserMessageSent` events independently: unrelated runtime-pallet changes do not invalidate these stable schemas, but incompatible target-event metadata still HOLDs. Shard allocation retains each ordered message separately even when payloads repeat.

Existing active/initializing programs require exact finalized readback, not another constructor. Legacy pending constructors without signed bytes may adopt an exact existing finalized program but cannot claim an original receipt; otherwise they HOLD. Legacy signed configuration calls gain a genesis scan anchor only after raw signer/nonce/call validation. Consumed nonces, expired signatures, missing archive history or conflicting intent HOLD without replacement. Code uploads are attempted only after finalized storage explicitly reports absence; all unresolved saved messages settle before another upload/action can consume the signer nonce. Program status writes merge into the latest journal rather than overwrite recovered evidence.

Use `ethereum/script/BeefyTokens.s.sol:BeefyTokens` and the shared [deployment instructions](../ethereum/README.md#beefy-deployment-and-regression-checks). Set the following environment inputs from the saved anchor, not a newer unrelated RPC head:

| Input | Source |
|---|---|
| `BEEFY_SOURCE_DOMAIN`, `BEEFY_BRIDGE_DOMAIN` | Original source domain and destination binding above |
| `BEEFY_MMR_START_BLOCK` | `anchor.mmrStartBlock` |
| `BEEFY_INITIAL_BLOCK`, `BEEFY_INITIAL_SOURCE_TIMESTAMP_MS` | `anchor.block`, `anchor.sourceTimestampMs` |
| `BEEFY_CURRENT_ID/LENGTH/ROOT` | `anchor.current.id`, `anchor.current.keys.length`, hex-encoded `anchor.current.root` bytes |
| `BEEFY_NEXT_ID/LENGTH/ROOT` | `anchor.next.id`, `anchor.next.keys.length`, hex-encoded `anchor.next.root` bytes |
| `EXPECTED_DEPLOYER_NONCE` | Reserved original deployment nonce |
| `GEAR_VFT_MANAGER` | `token-stack.json.programs.vftManager.id` |
| `BEEFY_RECOVERY_TIMELOCK` | Independently approved deployed controller; recorded address, code hash and positive delay |

Also configure the actual governance admin/pauser, emergency-stop roles and positive bridging-payment fee. Simulate before an authorized broadcast, retain the Foundry journal, and reconcile original receipts/nonces before retrying partial deployment. Retained-queue migration preserves existing source UUPS authority while explicitly binding the reviewed timelock's narrow recovery capability; a placeholder Safe/controller is not approval. See the [migration procedure](../ethereum/README.md#mainnet-custody-preserving-migration-candidate).

Keep a coherent task-owned Foundry output/cache for the token script and set `FOUNDRY_OUT` to the same `--out` directory for OpenZeppelin FFI validation. Mixing full-build and partial-script build-info can produce duplicate-contract validation failures; do not disable upgrade validation or overwrite prior approved artifacts to work around them. Retain the successful simulation and verify every original sender, nonce, calldata and CREATE address before accepting canonical finalized deployment receipts.

Once the original deployment is finalized, set `CLIENT`, `VERIFIER`, `MANAGER` and `QUEUE` to those contract addresses. Use the deployment wallet's protected JSON file, not a key in command arguments:

~~~sh
target/release/beefy-deploy tokens-manifest \
  --source-rpc "$SOURCE_RPC" --witness-rpc "$WITNESS_RPC" \
  --ethereum-rpc "$HOODI_RPC" --ethereum-witness-rpc "$EL_WITNESS_RPC" --wallet "$DEPLOY_WALLET_JSON" \
  --anchor "$RUN/anchor.json" --token-stack "$RUN/tokens/token-stack.json" \
  --client "$CLIENT" --verifier "$VERIFIER" --queue "$QUEUE" --manager "$MANAGER" \
  --output "$RUN/deployment.json"
target/release/beefy-deploy tokens-configure \
  --source-rpc "$SOURCE_RPC" --witness-rpc "$WITNESS_RPC" \
  --expected-genesis "$SOURCE_GENESIS" --gear-suri-file "$GEAR_SURI_FILE" \
  --ethereum-rpc "$HOODI_RPC" --ethereum-witness-rpc "$EL_WITNESS_RPC" --wallet "$DEPLOY_WALLET_JSON" \
  --deployment-manifest "$RUN/deployment.json" --token-stack "$RUN/tokens/token-stack.json"
~~~

Configuration authenticates the original deployment, programs, mappings, decimals, roles and native wrapper binding before resuming. Its journal preserves completed actions. Never override manifest identity or replace an unresolved signed operation.

### 3. Run and verify

Use distinct funded commitment and root signers, separate from deployment and other workers:

~~~sh
target/release/beefy-relay tokens-follow \
  --source-rpc "$SOURCE_RPC" --witness-rpc "$WITNESS_RPC" \
  --ethereum-rpc "$HOODI_RPC" --ethereum-witness-rpc "$EL_WITNESS_RPC" \
  --wallet "$FOLLOWER_WALLET" --root-wallet "$ROOT_WALLET" \
  --deployment-manifest "$RUN/deployment.json" --output-dir "$RUN/follower" \
  --client-deployment-transaction "$CLIENT_DEPLOYMENT_TRANSACTION"
~~~

Use the original client CREATE transaction hash from the retained deployment journal. This is only a locator: the follower still authenticates finalized ancestry, transaction/receipt roots, the signed CREATE address, approved creation bytecode and constructor bindings before retaining the bootstrap pin. A conflicting restart hint holds. Without this hint, discovering creation requires historical `eth_getCode` state, which a pruned public RPC may not serve.

Use an archival execution endpoint for `HOODI_RPC`: public Hoodi accepts HTTPS or WSS and preserves the requested transport; owned fixtures still require their approved loopback WebSocket endpoints. Both transports bound individual requests to 30 seconds. The CREATE locator does not remove the archive requirement: restart must reauthenticate historical client/queue state and original inclusions. A pruned-state error is HOLD, not permission to reset the journal or bypass validation. Before restarting on another same-network archive endpoint, run the same command and original directory with `--reconcile-once`; resume continuous following only after that succeeds.

Run one writer per durable follower/root journal and signer. Independent permissionless publishers may advance the same contracts; reconcile their authenticated progress without relabeling original submissions. Start checkpoint, inbound and paid outbound workers with the retained `relayer` commands above, the same deployed identities and separate durable directories/signers. Start application traffic only after authenticated catch-up, checkpoint readiness and all independent proof-profile approvals.

For a fresh queue, pass its original direct CREATE transaction through `--queue-deployment-transaction` (or `QUEUE_DEPLOYMENT_TRANSACTION`) to the outbound worker. The locator is not a trusted scan floor: the worker authenticates the finalized block, full transaction/receipt roots, signed CREATE address and chain/genesis identity, then replays inclusively from the queue birth block. Restart reauthenticates the saved birth; conflicting hints or already-progressed legacy journals HOLD. Only an untouched legacy rebuilding journal can adopt the authenticated birth atomically. Omit the locator to retain genesis replay; never reset an existing cursor or delete pending finality fences.

Paid inbound event readers preserve typed HTTP errors: a read-only provider 429 is recoverable, while authentication failures remain terminal. Resume the same discovery and transaction journals after an outage; this does not authorize retrying an ambiguous signed submission with new bytes.

Verify original queued messages, finalized commitment/root/delivery receipts, maturity, replay protection and exact recipient/escrow/supply effects. Native queued/mailbox value is not settlement. The SDK [examples](../js/bridge-js/README.md) preserve original intent and operation deadlines; full live tests require the independently approved proof profile and owned source, execution, Beacon and proxy fixtures. Unit mode is not a substitute. Frontend completion requires settlement, not merely a successful relay submission.

Each manual UI claim starts one 44-minute absolute deadline before wallet/archive setup and requests SDK waiting. Stale indexer availability is advisory, not a signing prerequisite; missing or malformed independently approved inbound profiles remain HOLD. Approval is immutable while authenticated preparation observations may advance only from `NotPresent`; outdated checkpoints, identity changes and missing/forked history hold. The SDK rechecks original, previous and final hash pins across Beacon I/O.

For historical outbound backfills, the selected source height/message/proof stays fixed while unsigned simulation waits only on the decoded `MerkleRootDelayNotPassed` error. Each retry reauthenticates finalized queue root and anchor and uses the same deadline; signing occurs once after maturity. Other failures are immediate. Preserve any known original Gear/Ethereum transaction hash in HOLD and reconcile it before retrying; an unresolved remote wallet request has no invented hash or cancellation guarantee. Only authenticated token/native settlement triggers UI completion.

Offline periods require catch-up and reconciliation of original signed work, not re-genesis, fresh nonces or reset deadlines. No uninterrupted timed soak is required. The follower retains completed history indefinitely, rewrites its entire JSON journal, and reauthenticates retained commitments within a 300-second startup deadline. Qualify disk growth and restart time for the intended operating period; no automatic history compaction is implemented.

Manager replacement uses an authenticated Beacon-slot cutover, not replay-key export/import. Keep the original manager and its evidence queryable; retain the same native wrapper. Resolve lifecycle blockers on all canonical pre-boundary receipts, original signed work, native payouts, outgoing requests and definite refunds before custody transfer. A terminal receipt exclusion is not financial settlement: held native liabilities remain at the wrapper. Authentic unpaid deposits requiring a new submission need explicit `--sponsor-unpaid`; already-authenticated exclusions need no new signature or fabricated fee evidence. Keep the replacement paused through source retirement, token/wrapper role handoff and finalized readiness. See the [manager procedure](../gear-programs/vft-manager/README.md).

#### Mandatory manager-replacement inventory gate — accepted P0-A3 availability risk

Ordinary non-native mint/transfer errors do not prove no effect. Their `Retryable` or ambiguous reservations can permanently block cutover; there is no non-native admin-clear, terminal/held escape, or universal non-native 80-bit admission guard. Native replay safety relies on wrapper operation-ID deduplication, which ordinary token calls lack. The following procedure is mandatory **before `BeginCutover` and again at the paused/drained boundary before the first custody `Upgrade`**:

1. Authenticate the source program/code identity and its actual query ABI. Pin one canonical finalized Gear hash and the complete Ethereum/source admission inventory through the intended boundary, including paid, unpaid, public, administrative and benchmark admissions. Retained canonical coverage and original journals are evidence, not proof that omitted admissions do not exist.
2. Establish an independently authenticated **complete reservation/admission inventory**. There is no public query enumerating `RESERVED_TRANSACTIONS`; `Transactions`/`ReadTransactions` expose successful history only. A journal-only or indexer-only scan is insufficient. If completeness cannot be established for the deployed program, record **HOLD**, never an empty scan. This accepted procedure adds no scan CLI or API.
3. For every inventoried receipt key, query `ReceiptStatus(slot, transaction_index)` and `ReceiptDeposits(slot, transaction_index)` at the same pin using the actual deployed ABI. Any non-native pending, in-flight, retryable or unknown outcome blocks. Reserved keys with absent/malformed rows, missing archive/API or incomplete inventory also block. Record source code ID, pin, key/log, operation ID, child, token/supply, amount and outcome; recheck canonicality.
4. Scan historical source `BridgingRequested` events for zero recipient/amount and native amounts `>= 2^80`. Retain each source economic liability separately from target rejection; an Ethereum upgrade does not invalidate old receipts. The native representable maximum is `2^80−1` base units (about **1,208,925.8196 tokens only at 18 decimals**). Recipient balance capacity and token minimum balance are separate limits.
5. If unresolved state is found before beginning, do not call `BeginCutover`. If found afterward, retain the same `Draining` source and original evidence. Repair recoverable configuration and use existing recovery rules; permanent ambiguity or unrepresentable input requires a separately approved liability-preserving solution. Never force-clear reservations, bypass quiescence, reset journals or transfer custody around the block.

A point-in-time active scan cannot prevent later admissions; the repeated paused scan and onchain quiescence gates remain authoritative. These are **manager-replacement** requirements, not a requirement to replace Vara programs for retained ZK→BEEFY verifier migration. That separate migration preserves the queue proxy, ERC20 manager, tokens and Vara programs; qualify its atomic paused upgrade and register needed legacy roots before stopping old ZK submissions, as specified in `ethereum/README.md`.

#### Exceptional liabilities are not financial settlement

Pre-upgrade native amounts `>= 2^80` become `Rejected(NativeAmountOverflow)` without a target child. This prevents an invalid target dispatch; it does **not** reverse an already-mined Ethereum burn/escrow. `Resolved` is not repayment. Retain these and zero-input source liabilities as deployment gates. Historical `NativeErrorAbsent` tombstones remain decodable financial evidence but cannot authorize new manager resolution or relayer exclusion; this remediation neither erases nor repays them. A transient authenticated wrapper error stays retryable with the same economic operation ID, while public reconciliation never creates a missing payout.

Native lifecycle commands with a unit result use the pinned Sails 0.9.2 empty automatic success reply; nonunit replies still require their exact route and complete SCALE decoding. Discovery rejects malformed or unadmitted public Redirect/proof/reconciliation candidates without treating them as retained-evidence corruption. For financial settlement, only an authenticated successful or pending-native original can own an external admission: an unpaid self-relay rejected while paused does not prevent explicit sponsorship. Terminal exclusions use a separate authenticated original-call and resolution path, not arbitrary application errors. After a role call finalizes before its reply is saved, resume the same cutover work: the tool reauthenticates and persists that original before checking successor role readback. Live role state alone never proves original success or authorizes a replacement signature; retained bytes, nonce, reply and canonical history remain strict.
External discovery infers native reconciliation ownership only for an admitted pending-native parent, and only through its first canonical successful `ReconcileReceipt` reply reporting `Processed`. Redundant public reconciliation after successful parent or native completion, including later failed calls, cannot replace that original or delay cutover. Earlier retained terminal attempts and the exact successful continuation remain authenticated alongside full historical economic settlement; malformed retained evidence and unresolved earlier originals still HOLD.
Pending external admissions from older discovery may refresh only their inferred public continuation projection after canonical rediscovery authenticates the exact unchanged parent signature, inclusion, message, reply and provenance. The saved parent discovery pin is still reauthenticated; settled admissions and locally owned signatures/attempts retain strict evidence-preservation checks. Completion requires full historical settlement at or after retained original replies; bounded row eviction or missing archive still HOLDs, and nonce order does not establish which read-only reconciliation caused completion.

For an explicit native liability hold:

1. Pause the source manager and wrapper through their respective admins. Query the exact original `ReceiptDeposits` row and wrapper `Redemption(operation_id)`.
2. Have the wrapper admin submit `HoldRedemption(operation_id, expected, acknowledge_irreversible_hold)` with that unchanged non-terminal snapshot. A `Queued` payout or one with backing at least the amount requires explicit `true` consent: this **irreversibly disables wrapped-credit retry**, not debt forgiveness or payment. A partial ambiguous payout may use `false`. The original callback can still arrive. Then have the manager admin submit `ResolveReceiptDeposit` with the exact receipt/log/operation and original wrapper-request child; both the saved hold and live payout must still be non-terminal. A late delivery/refund is economic evidence, not a new active unpaid hold. No-effect rejection needs no wrapper hold; ambiguous non-native effects cannot be force-cleared.
3. Verify the receipt is `Resolved`, its exceptional row is `Held`/`Rejected`, and `BlockingRedemptions` is zero. Retain `OutstandingRedemptions` and saved hold snapshots as financial evidence; a held payout can remain outstanding. Already-finalized manager `Held` rows are historical exceptions, not automatically rewritten after a late callback. Resolution waits for an expired/released receipt lease and fails closed on changed or missing evidence. **P0-R1 remains open:** there is no held-liability release/payout API.
4. Resume the wrapper before source custody transfer, keeping the source manager paused. Resume the normal prepare/finalize/verify workflow; role handoff pauses the wrapper again when rotating its manager and burner.

Delivery can race the manager's two wrapper reads or complete in the same block
as a `Held` resolution. Such a manager row remains a historical exception.
The relayer separately authenticates its saved `Queued`/zero-return hold and
the exact original `Delivered`/zero-return payout at one finalized pin; this is
exclusion authorization, not an active unpaid hold or financial-success rewrite.
Missing or changed identities, children or backing still mean HOLD. `Refunded`
is not accepted by this historical-delivery path: the qualified wrapper cannot
mint after a hold. Previously saved exclusion pins remain pinned to their
original finalized state; no journal or receipt history is rewritten.

These commands only print generated routed SCALE payloads; they never broadcast. The hold input is raw SCALE `Redemption` without a route prefix, optionally prefixed with `0x`. The manager child is a trailing optional positional argument (omit it only when the original row has no child):

~~~sh
target/release/vft-manager-tool hex-encoded-message hold-native-redemption \
  "$OPERATION_ID" "$RAW_REDEMPTION_HEX" --acknowledge-irreversible-hold
target/release/vft-manager-tool hex-encoded-message resolve-receipt-deposit \
  "$SLOT" "$TRANSACTION_INDEX" "$LOG_INDEX" "$OPERATION_ID" "$ORIGINAL_MANAGER_CHILD"
~~~

Omitting `--acknowledge-irreversible-hold` encodes `false`; it does not silently consent to disabling a resolvable payout. Use only an independently qualified deployed ABI: newer payloads cannot retrofit immutable live programs.

New work, coverage, lifecycle reports and seals use schema version 2. Each terminal admission has exactly one historical settlement or exclusion pin. Exclusions authenticate the complete receipt cohort, canonical transaction/receipt tries and exact transaction index/hash, plus every retained parent and native-continuation nonce disposition no later than exclusion and source retirement. An enqueued parent still needs its original finalized reply. Owned excluded rows remain in the original active journal, never in successful completion history; work retains an authenticated copy without rewriting original bytes. Foreign exclusions use `externally-resolved`; paid/sponsored fee evidence stays unchanged. Restart and read-only verification reauthenticate exclusions, not just file digests.

Prepare, finalize (including an already-activated retry), and draining-error
recovery release cutover signer ownership only after authenticating every
retained original. A terminal exclusion qualifies separately from settlement:
its signed work and active journal row remain intact. Pending or conflicting
settlement/exclusion evidence, or a failed historical authentication, retains
the signer hold; never remove ownership files manually to bypass it.

Mutable version-1/2 work upgrades to version 3 only after its historical role pins, source identity, saved coverage and checkpoint continuity authenticate. Work and coverage require a non-null execution checkpoint; the seal binds the immutable coverage through its digest. Immutable version-1/2 predecessor seals and reports stay byte-for-byte unchanged and are reauthenticated on every startup, existing-seal retry, already-Active retry and read-only verification. Genuinely missing legacy coverage inventories are reconstructed in memory; malformed/partial inventories or absent source-domain fields HOLD. Zero blocking can be inferred for an old wrapper only from zero outstanding; missing historical evidence holds. Version-1 restrictions and settlement XOR exclusion remain; Held/Rejected are not successful completion. This does not retrofit already-deployed immutable Gear programs.

Mutable scanning commits each covered block, its cursor and **all** derived admissions in one atomic work-file save before resolving any admission. Restart also reconciles retained admissions when the boundary was already committed. Recovery authenticates saved coverage locally and fetches bodies only for missing admitted blocks; it does not advance the scan cursor past unrecorded sibling deposits. Empty legacy mutable inventories must still scan from genesis. Immutable predecessor coverage reconstruction remains complete from genesis through the sealed boundary; the mutable recovery optimization never certifies a partial seal.

Repeated upgrades and read-only verification preserve the drained-wrapper inference only when the historical outstanding count is zero and migrated work retains a zero blocking count. A missing getter never excuses nonzero, unknown or contradictory liabilities; historical pin and saved-field comparisons still apply on already-Active retries.


Seal the exact old transaction, block, discovery and completed coverage/admission bytes in immutable `cutover.json`; install matching `.cutover-seal` markers while old writer locks remain held. Normal relayers must not reopen sealed lanes. Only read-only `verify-cutover` may reauthenticate their original evidence. Successor journals are new paths bound to that seal and the approved new receiver, beginning at execution block B+1 and receipt slot S+1. An incomplete handoff or unavailable original history is HOLD, not permission to relabel the old target.

Stop both old lane writers before preparing the cutover; the tool exclusively locks their original journals. `OLD_TRANSACTIONS` is the transaction directory (including `state.json` and `blocks.json`); `OLD_DISCOVERY` is the discovery JSON file. `NEW_MANAGER_CODE_ID` must come from independent approval of the unused, manually paused replacement. Keep the same paths, boundary and identities on every retry:

~~~sh
target/release/vft-manager-tool --gear-endpoint "$SOURCE_RPC" prepare-cutover \
  --old-vft-manager "$OLD_MANAGER" --new-vft-manager "$NEW_MANAGER" \
  --new-manager-code-id "$NEW_MANAGER_CODE_ID" \
  --old-transactions "$OLD_TRANSACTIONS" --old-discovery "$OLD_DISCOVERY" \
  --ethereum-rpc "$EL_RPC" --ethereum-witness-rpc "$EL_WITNESS_RPC" --beacon-rpc "$BEACON_RPC" \
  --boundary-slot "$S" --cutover-dir "$CUTOVER_DIR"

target/release/vft-manager-tool --gear-endpoint "$SOURCE_RPC" finalize-cutover \
  --cutover-dir "$CUTOVER_DIR" \
  --new-transactions "$NEW_TRANSACTIONS" --new-discovery "$NEW_DISCOVERY" \
  --ethereum-rpc "$EL_RPC" --ethereum-witness-rpc "$EL_WITNESS_RPC" --beacon-rpc "$BEACON_RPC" \
  --evm-registration-transaction "$REGISTRATION_TX"

target/release/vft-manager-tool --gear-endpoint "$SOURCE_RPC" verify-cutover \
  --seal "$CUTOVER_DIR/cutover.json" \
  --ethereum-rpc "$EL_RPC" --ethereum-witness-rpc "$EL_WITNESS_RPC" --beacon-rpc "$BEACON_RPC"
~~~

Supply the authorized Gear lifecycle signer through protected `GEAR_SURI` for prepare/finalize; the CLI development-key default is not an operator identity. `verify-cutover` neither uses a signing credential nor changes journals. Add `--sponsor-unpaid` to prepare/finalize only with explicit authorization to pay for authentic unpaid deposits. Otherwise unresolved unpaid receipts remain HOLD; previously fulfilled external receipts require their own authenticated original and settlement evidence, never borrowed lane provenance.

Preparation leaves the source `Draining`. Finalization resumes original custody and role calls, requires the actual finalized authorized EVM successor-registration transaction while retaining the old registration, seals the historical lane, initializes the new lane at B+1/S+1, and only then activates it. The tool has no EVM signer. Missing registration is HOLD after custody/role handoff; perform that authorized registration externally and resume the same directory. Do not restore or reuse old lane files to bypass a HOLD.

Before beginning the source drain, direct and paid lane identities must bind the same Ethereum emitter as the cutover manifest. A stale emitter identity is rejected before `BeginCutover`, not deferred until successor startup.

Retain `cutover-work.json`, `cutover-coverage.json`, `cutover-calls.json`, `cutover.json`, `cutover-activation.json`, original journals and matching seal markers. Start the successor with the tool-reported `--cutover-seal` and `--successor-manager-code-id`; a retry after activation authenticates the original activation and preserves successor progress. If the activation marker was not saved, finalization first reconciles the original unpause call and validates existing successor journals, even when populated; it never reinitializes them or signs another unpause for an ambiguous original.

Indexer scheduling confirmations are not finality. Both processors and the repair command require `ETH_RPC_URL`, independent `ETH_WITNESS_RPC`, approved decimal `ETH_CHAIN_ID`, and exact 32-byte `ETH_GENESIS_HASH`. Both EL URLs require HTTPS except owned loopback HTTP fixtures. `ETH_HTTP_RPC_URL` remains metadata-only, never an implicit witness. The Gear processor also requires `ETH_ERC20_MANAGER` to authorize original source receipts. Repair needs that manager and `ETH_MSQ_QUEUE`, but no Squid/archive configuration. Every consumed manager, queue and payment event must match its independently qualified receipt. Gear settlement completion additionally requires that exact original EL deposit; this adds no new Gear trust model.

### Indexer evidence migration and bounded repair

Pause **both processors and the API**, retain a database backup, install the upgraded code/configuration, and apply `1792540800000-ethereum_evidence.js` through the existing migration runner:

~~~sh
yarn build:indexer
yarn workspace indexer migration:run
~~~

Only resume upgraded processes. The single migration transaction preserves legacy resolution coordinates/flags and original receipt/deposit JSON, demotes unauthenticated Vara→Ethereum Completed and Ethereum→Vara Completed/Refunded projections to Bridging, clears their live completion coordinates, and resets unqualified matching flags. This includes old direct completions without an observation. Other directions, amounts, source identities, nonterminal/Failed rows, role maps and native/refund children are untouched. NULL authorization never qualifies an outcome. Rollback refuses to discard preserved or qualified evidence.

After invalidation, refresh already-open clients: their completed-transfer polling may have stopped. Claim visibility requires `completedAt == null` and AwaitingPayment/Bridging; the API may resume while bounded repair restores independently authenticated outcomes. Preserve the separate [event-coordinate reindex requirements](internals.md); migration alone does not authenticate old rows.

~~~sh
yarn workspace indexer reconcile --direction vara-to-eth --limit 1
yarn workspace indexer reconcile --direction vara-to-eth --limit 1 --apply
yarn workspace indexer reconcile --direction eth-to-vara --limit 1 --apply
# Continue the selected direction using the returned lastExaminedId:
yarn workspace indexer reconcile --direction vara-to-eth --limit 1000 --after "$LAST_EXAMINED_ID" --apply
~~~

Standalone reconciliation owns migrated, unauthenticated outbound observations: a non-NULL `legacyMatched` (true **or false**) with NULL `receiptAuth` excludes that row from the live pending page before pagination. A pruned legacy receipt therefore cannot block unrelated live batches. Current-batch events, unmarked observations and previously authenticated observations remain fail-closed; the marker never bypasses their qualification.

Each invocation examines one keyset-ordered page (default 1000, allowed 1–1000) and reports `lastExaminedId`, `hasMore`, qualified/pending counts and held IDs. Default is reporting-only: migrations are disabled, the transaction is read-only, and no checkpoint/projection/cursor is written. `--apply` uses one SERIALIZABLE transaction. Missing counterparts advance only the external scan hint, not verification. Outage, conflicting evidence or serialization failure aborts the entire page without a success cursor. Retain `heldIds`, restore the original historical evidence and rerun that page. To schedule unrelated later rows, explicitly use `--after` with the reviewed held ID and retain the failed row for repair: skipping never authenticates it or suppresses its claim. Concurrent checkpoint insertion never overwrites the winner; live ingestion retries its complete transaction, while repair aborts its page.

Periodic reruns include already-matched/terminal rows and revalidate the original receipt, effects, settlement/completion binding and coordinates. New finality may advance without replacing original authorization/checkpoint references. Refunded remains distinct from Completed. Legacy direct outbound recovery uses its preserved original completion hash/block, never the Gear source transaction hash or an arbitrary same-nonce success. Missing original evidence remains pending/HOLD, never an inferred delivery.

If original history cannot be recovered, leave the row pending/HOLD. Never delete its observation, clear its legacy marker, replace its original hash or synthesize authorization to unblock indexing. `legacyResolution` remains exposed historical, unauthenticated metadata; neither it nor `legacyMatched` is proof of delivery. Consumers must use the current qualified projection, not the preserved legacy claim.

Phase 1 does not close the Phase 0 immutable-program retrofit or held-liability payment gates, nor establish live-cutover qualification. Local dual-RPC execution/database fixtures exercise disagreement and preservation behavior, not production provider independence.

Gear program IDs from configuration seed missing database roles only. Authenticated rotation events update the durable role map sequentially; reloads and restarts preserve those rotations rather than restoring stale environment IDs.

GitHub-hosted live checks need a stable operator-owned authenticated read-only RPC ingress for the approved archive fixture. Do not expose development RPC directly: even safe RPC can submit extrinsics. Keep access URLs in protected secrets, independently approve fixture-profile changes, and provision durable ingress rather than relying on an expired temporary tunnel. Local live checks and public CI reachability are separate evidence.
The retained gateway is `js/bridge-js/test/readonly-rpc-gateway.mjs`; run its existing boundary check with `node js/bridge-js/test/test_readonly_rpc_gateway.mjs`. Its CLI requires a loopback `--upstream`, local `--port`, and protected `--token-file`. Terminate TLS at the approved ingress and store the authenticated `/rpc/<token>` URL only in the CI secret. This helper is CI infrastructure, not a campaign supervisor.

### Explicit isolated consensus fixtures

An owned integration fixture may use real execution, Beacon and validator clients with a separate `LocalFixture` checkpoint identity. This is not `--local-rehearsal`: BLS signatures, committee proofs, canonical receipts, finality and maturity checks remain enabled. Existing public-network identities and approval files remain unchanged.

Independently review the offline execution/Beacon genesis artifacts and persist a descriptor with exactly `schemaVersion: 1`, `ethereumChainId`, `ethereumGenesisHash`, `beaconGenesisValidatorsRoot` and `beaconGenesisTime`. Hashes are 32-byte `0x` strings; chain ID and genesis time are positive integers. Do not learn these trust pins from the RPC endpoint being authenticated. The fixture uses the mainnet preset (12-second slots, 32-slot epochs, 256-epoch sync periods, 512-member committees), Electra from epoch zero with fork version `0x60003137`, and no Fulu activation.

Use `checkpoints-tool --owned-fixture-genesis-time` together with the independently reviewed genesis validators root and bootstrap root. Its source and Beacon endpoints must be loopback. The initial signed finalized update must still be in the bootstrap period; period-zero initialization is not relaxed. Advancing the declared genesis clock does not authorize editing a state slot or injecting finalized checkpoints.

Supply `--owned-fixture-network /protected/offline-network.json` to `tokens-manifest`, `tokens-configure`, `tokens-follow`, `tokens-publish-root` and `tokens-history-audit`. Source, witness and execution endpoints must use numeric loopback addresses. The manifest persists this descriptor separately as `ownedFixtureNetwork`; every restart requires exactly the same explicit approval. Missing approval, changed pins, public endpoints or a conflicting rehearsal flag HOLD before signing. Omit the flag for the unchanged default Hoodi path.

Full SDK acceptance additionally requires `OWNED_FIXTURE_NETWORK_PATH` and a separately reviewed matching inbound proof profile; the frontend accepts the same descriptor through `VITE_OWNED_FIXTURE_NETWORK` only in its testnet configuration. A genesis descriptor is not approval of program code, deployment addresses or a new consumer profile. Keep fixture keys and journals owner-only, do not reuse public-chain keys, and report owned-fixture evidence separately from public-network qualification.

For RPC-only ingestion, set `ETH_RPC_ONLY=true` and `GEAR_RPC_ONLY=true` explicitly. This supports an independently approved public Hoodi endpoint as well as owned archive nodes without a Subsquid gateway; default gateway behavior remains unchanged. Ethereum logs are derived from transaction receipts, avoiding unfiltered `eth_getLogs` requests rejected by some public providers. Finalized-hash authentication, receipt checks and the no-partial-batch-commit barrier still apply. Use separate database state and real deployment floors; never point an isolated fixture at a public-network archive.

For a native local GraphQL process, set `GQL_HOST=127.0.0.1` and an available `GQL_PORT`; the default remains `0.0.0.0` for container ingress. Reuse an owned PostgreSQL service with isolated database/schema state for the new deployment, retaining prior network evidence and rows. A reachable schema is not proof that either processor indexed finalized transfers.

Native v2.1 event readers use hash-pinned metadata and strictly decode the Gear/bridge events they consume. Unrelated runtime-pallet or dispatch-info changes must not break those reads; incompatible consumed schemas, missing failure fields, original identity mismatches and noncanonical pins remain errors. Before native initialization, optional `QueueMerkleRoot` is absent, not an initialized zero root; discovery authenticates this distinction on both nodes and keeps initialized-root reads strict. Checkpoint updater writes use the same targeted original inclusion/reply checks rather than whole-runtime event decoding.


## First-start checklist

Before allowing the service to submit transactions:

1. Verify source and witness endpoints can serve the approved finalized history, BEEFY commitments and MMR/snapshot evidence; retain GRANDPA support for consumers that require it.
2. Verify the archival Ethereum RPC's approved chain/genesis, finalized state and original transaction/receipt evidence.
3. Verify queue/client/verifier bindings, source/MMR/bootstrap identities, recovery timelock and independently approved reverse proof profile.
4. Verify each process has its own signer and private durable storage, with historical stores and original signed evidence retained.
5. Render the supervisor configuration and inspect every path, secret source, listener and metrics address.
6. Reconcile the same journals before continuous following; confirm authenticated catch-up and inspect logs/metrics rather than relying on process exit alone.
7. Only then enable delivery and application traffic. Source changes alone do not qualify or activate a public deployment.
