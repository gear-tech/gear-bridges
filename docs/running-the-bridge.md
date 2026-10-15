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
- Go, for legacy prover builds using [gnark-wrapper](../gnark-wrapper), not standalone BEEFY builds.
- Foundry v1.8.5 (forge/cast, matching CI) for the Ethereum contracts and deployment tooling.
- The native build dependencies listed in [Dockerfile](../Dockerfile), including a C compiler, OpenSSL development files, CMake, protobuf compiler, and Clang.

The root README also calls out the [ring build instructions](https://github.com/gear-tech/ring/blob/main/BUILDING.md). Follow those instructions when a native build fails while compiling `ring`.

The Go module pins the original ignition-verifier commit through a public mirror because the upstream repository is unavailable; the cryptographic implementation and revision are unchanged.

Build the relayer from the repository root:

~~~sh
cargo build --release -p relayer
target/release/relayer --help
~~~

Standalone BEEFY development builds use `cargo build --release -p beefy-relay` and do not enable the legacy prover or its Go build. The `relayer` package still enables `legacy-prover` by default; its legacy CLI requires that feature. This does not change artifact qualification: token deployments must use the same locked all-targets release graph described below, not mix WASM from a package-only build with qualified deployment artifacts.

Proof generation uses large native stacks. `gear-eth-core` requires `RUST_MIN_STACK` to be set to at least 4 MiB before startup. A typical native invocation starts with:

~~~sh
export RUST_MIN_STACK=4194304
export RUST_LOG='relayer=info,prover=info,ethereum-client=info,metrics=info'
~~~

Increase `RUST_MIN_STACK` or reduce proof worker counts when a host is memory constrained. Each configured proof thread can allocate substantial memory.

### Native node regression tests

Run native integration cases against a fresh, owned local node matching the CI
image (`ghcr.io/gear-tech/node:v1.9.0`), with RPC on `127.0.0.1:9944`. Do not run
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

Each Ethereum fee-payer account must have exactly one submitting process. The in-process nonce lock is shared by clients in one relayer process, but it cannot coordinate replicas, manual relay commands, or external signer tools. Assign distinct fee-payer accounts to those writers, and do not run active-active replicas with the same `ETH_FEE_PAYER`.

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

For a transient RPC failure, the Gear listener reconnects and replays missing finalized blocks; Ethereum pollers reconnect and resume from their persisted cursors. The shared Ethereum WebSocket handshake and individual RPC requests time out after 30 seconds for `EthApi`, `PollingEthApi` and raw-provider calls. Transient receipt-evidence transport failures reconnect and recheck the original signed transaction; identity, finality and event mismatches remain HOLD. A timeout is not proof of non-submission and does not authorize replacement signing, a different nonce or changed deadlines. A zero process exit code does not establish worker health: a failed child task can close the pipeline without a nonzero exit. Inspect logs and metrics before deciding whether to retry, restore state, or escalate to the emergency procedures.

Wrapped HTTP transport errors are classified through their source chain: a
reqwest/hyper connection reset or an unreachable/down host or network must not
become permanent merely because its outer display says only “error sending
request”. Explicit permanent RPC/status
errors and ambiguous transaction-send failures keep their existing fail/HOLD
precedence. The regression drives actual finalized-block and deposit readers
against a loopback server that resets the socket after receiving the request.

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

Outbound schema-4 journals migrate versions 2/3 without replacing signed bytes or prior evidence. Completion still requires the original signed transaction's canonical finalized successful receipt, configured confirmation depth, exact queue event and same-receipt token effect. The sender alone retries an absent original using its exact saved bytes, including after restart in WaitConfirmations and when another caller processed the message. The read-only watcher never treats that processed bit as original success. A finalized original revert is a durable failed-original outcome: retain its full receipt/finality evidence, count it once, release account ownership only after saving, and continue unrelated work without automatically signing that message again. An absent original with an authenticated finalized consumed nonce is held, not failed or completed; save and reauthenticate its nonce disposition before releasing account ownership. Neither outcome acknowledges the original source/fee event pair.

Native recovery retains every original read-only reconciliation continuation and its authenticated terminal outcome before preparing a later continuation. Unknown replies remain HOLD; a continuation never resubmits the economic receipt or creates another payout. Completed rows are requalified at their saved finalized settlement pin, including historical manager mappings, receipt deposits and wrapper redemption state; bounded history eviction at the current head does not erase original completion evidence.

The native wrapper cannot exit, even after all payouts settle. Its constant-time outstanding count includes Queued, Returned and Ambiguous obligations; only the first authenticated Delivered transition decrements it. Manager rotation requires paused state and zero outstanding obligations. Definite owner-authorized refunds remain available while manually paused or emergency-stopped; ambiguous deposits and builtin outcomes remain fenced. Reconciliation queries use configured gas, reply deposit and timeout without changing an unknown payout child.

RPC absence, timeout, transport error or invalid finality never unlocks an unconsumed signed nonce. A permanently underpriced original can therefore fence its shared signer indefinitely; independent governance/escape signers remain the availability boundary. A retry does not allocate a nonce, sign again, create another watcher or extend an operation deadline. Manual terminal reports preserve the original hash and journal for reconciliation.

Token event listeners and paid-message requests use finalized Gear blocks without requesting a GRANDPA justification; static authority sets may not serve `grandpa_proveFinality` for such blocks. The `gear-eth-core` Merkle-root publisher still requires a real GRANDPA justification and fails closed when one is unavailable. Event-only reads are not a substitute for a root proof.

Keep each deployment's token-relayer journals under a dedicated storage path. The Ethereum-to-Gear receipt journal uses schema 5; an older journal fails to load instead of silently replaying a transfer. Preserve old journals for separate reconciliation; an isolated fresh deployment uses a new storage path and does not depend on repairing the retiring lane. Never relabel or transplant old journals. A held receipt (including an Unknown or unavailable finalized status) is not permission to submit it again; inspect the original payload and finalized VFT manager status. The journal retains the original receipt key, proof payload, composition/handoff times, and first submission reply or error with its timestamp and sender/program identities. Later finalized-status observations and query diagnostics cannot overwrite that evidence, including after completion or restart. Gear-to-Ethereum queued/paid cursors and observations must stay with its journal across restarts.

A schema-5 unsigned `SubmitMessage` with no handoff, or `PreparingSubmission`, resumes its original fully decoded receipt proof; a queued slot/index mismatch or trailing SCALE data holds before signing. Ambiguous unsigned handoffs remain status-only. Historical finalized-no-effect attempts remain archived when a genuinely retry-ready current attempt resumes; they are not confused with a new ambiguous handoff. Schema 4 and earlier do not establish the original-transaction binding guarantee and remain HOLD without rewriting their UUIDs, hashes or proof bytes.

Sender responses wake the shared inbound transaction manager instead of waiting for its 12-second idle reconciliation timer. Only prepared signatures and successful completions immediately advance queued work; unresolved responses do not immediately requeue themselves. Signing remains serialized, and prepared original bytes are still persisted before submission; a response does not authorize a replacement signature or skip finalized reconciliation.

New inbound attempts persist the exact signed Gear extrinsic, hash, account nonce, chain/program identities, receipt binding, and finalized scan checkpoints before broadcast. Restart reconciles or rebroadcasts those same signed bytes; it does not sign a replacement merely because receipt status is Unknown. A new attempt requires finalized no-effect evidence: a failed dispatch before message queuing, a complete canonical scan proving the original extrinsic absent through a finalized consumed nonce, or a retryable per-log VFT reply. Preserve every prior attempt. In a compatible schema-5 journal, a held entry without signed identity remains status-query-only; unsupported older schemas never enter the worker.

Even when the manager reports a receipt already processed, the worker first checks the persisted signed attempt against the current Gear genesis, sender, manager/proxy, receipt payload, and exact transaction hash. A coincident receipt key on another deployment cannot complete a mismatched journal. Before returning a proof, composition authenticates the full Beacon block against its checkpoint chain, binds the selected raw transaction hash and index, and verifies receipt metadata and the MPT value against the authenticated receipts root. A Processed shortcut cannot replace these checks.

Manual inbound relay requires a private, stable `--storage` directory with exclusive ownership. Reruns bind the same original Ethereum transaction/inclusion, receipt slot/index, route, deployment and Gear signer, and restore the shared durable sender before work starts. Its optional schema-5 `manual_identity` cannot be adopted by automatic workers. Changed identities or signed bytes (including completed and archived attempts) hold without rewriting evidence; successful completed reruns require historical and current canonical Processed observations. Never clear held storage to obtain a new signature. Before restoring any transaction, every persisted map key must equal its embedded UUID, and active/completed journals must not share a UUID; conflicting ownership holds without collapsing signed attempts into another transaction.

The sender reports a post-dequeue balance-RPC failure through the original request UUID rather than losing the consumed request. Run the retained live-RPC regression with `GEAR_BALANCE_FAULT_RPC=<read-only fault proxy> cargo test --locked -p relayer --lib message_relayer::eth_to_gear::message_sender::tests::post_dequeue_balance_fault_reports_original_request -- --ignored --exact --test-threads=1`. The proxy must pass the first `System.Account` storage read, fail the second with `balance-fault-after-dequeue`, and reject mutating RPC methods. This check prepares no signed output and broadcasts nothing.

The VFT manager accounts for every matching deposit log in an authenticated receipt, retaining per-log progress across retryable partial failures. Completed logs are not credited again; ambiguous mint/unlock effects remain held. Existing whole-receipt processed and reserved keys still block replay, so this change does not automatically backfill previously stranded logs in a legacy processed receipt.

The deployable full Cargo graph can enable VFT benchmark routes. All three benchmark mutations require the existing admin before sending messages or changing receipt history or token mappings. Qualify their authorization and accounting against that same release-profile WASM, not a feature-reduced replacement.

Inbound token workers restore receipt and block journals before starting listeners; unreadable journals stop startup. Backfill runs oldest-first and retains a block until processing succeeds, so another disconnect cannot advance the cursor past unfinished work. Restart with the same storage path rather than clearing it.

Inbound token workers require `--ethereum-blocks <discovery.json>` and `--ethereum-start-block <deployment-block>`. Discovery schema 1 pins the complete runtime identity, original deployment start, contiguous discovered/extracted block-hash cursors and pending transaction handoffs. Keep it paired with transaction state and separate from the transaction directory’s `blocks.json`. Restart authenticates cursor/pending coverage and replays only pending work before scanning new blocks; bounded channel backpressure replaces all-history height replay. Each handoff is durable before extraction is acknowledged and is cleared only after transaction intent is durable. Unbound legacy arrays, a changed start/identity, gaps, missing original hashes, incomplete journal pairs or interrupted snapshots HOLD unchanged; never bump the start to bypass them. Completed proof evidence remains retained: real transitions still persist it, but idle ticks no longer rewrite the full history.

The BEEFY publisher persists its transaction intent, source block identity, and nonce before broadcast; after an uncertain send it reconciles the same transaction and canonical contract state instead of choosing a new nonce. Fresh commitment records include the source block number before the first nonce-reservation checkpoint. Restart selects that exact reserved source block rather than an earlier eligible handover; an archive that passes it without finding it holds the lane. Saved signed bytes are decoded completely and checked against their hash, sender, nonce, and destination. RPC transaction lookups decode only the required identity and optional inclusion fields, so a missing `accessList` does not prevent recovery; malformed or conflicting identity still holds the operation. Canonical receipt and checkpoint verification remain mandatory. Keep root publication and token relayer signing keys separate. A conflicting root or source reorganization requires operator investigation; do not discard the journal to force progress.

Follower schema 4 separates commitment provenance: `origin: "own"` retains `submission`; `origin: "external"` retains the actual accepted signed `acceptedTransaction`. First attachment may adopt progress beyond the authenticated deployment bootstrap, but only after ordered source authority handovers and canonical finalized transaction/receipt proofs are verified. A log or current checkpoint alone is insufficient. `lastSuccessfulUpdate` remains own-success accounting; authenticated external progress may advance finalized/observed progress without completing an original transaction. Signed originals retain nonce ownership until their own finalized receipt or consumed-nonce disposition is durable; terminal originals move atomically into `terminalSubmissions` with complete original bytes and evidence. Competitor hashes never replace originals. Changed unfinalized inclusion requires independent finalized-header, transaction/receipt-trie, source-proof and accepted-checkpoint authentication; `firstInclusion` remains retained and finalized history cannot move. Idle same-set commitments are skipped; authenticated handovers, uncovered root demand and the 12-hour heartbeat drive paid updates. Readiness uses authenticated source age and the onchain 24-hour expiry, not an obsolete 64-block lag policy.

On follower restart, saved finalized commitments are revalidated against their original signed identity, receipt inclusion, finalized anchor and source block before any new submission. A finalized boolean is not sufficient. The worker uses one fresh verified finalized view, deduplicated targets and bounded 32-way checks. Durable hash-linked RLP headers and transaction/receipt trie evidence are locally authenticated on reload; incomplete ancestry fences and interior progress survive interruption beyond the unchanged 32,768-header memory bound. History revalidation and pending-transaction reconciliation share one 300-second startup deadline. Changed or incomplete evidence holds instead of skipping history, resetting the journal or extending warmup deadlines.

`beefy-relay tokens-history-audit --source-rpc <source> --witness-rpc <witness> --ethereum-rpc <EL> --deployment-manifest <manifest> --follower-state <existing-state.json> --proof-dir <new-private-audit-directory>` exercises that read-only historical authentication path without a signer. Keep the original journal untouched and use a separately owned proof directory, never the running follower’s archive. Repeat with the same audit directory for the warm-recovery check; fresh deployment history is not evidence for aged-journal recovery.

The Hoodi CLI default is `https://hoodi.drpc.org`. Its hash-pinned historical state and canonical receipts were checked against the retained v2.1.0 deployment; the read-only history audit authenticated all 693 originals in 58.1 seconds cold and 2.0 seconds warm without changing the follower journal. This qualifies that observed history, not a provider SLA or a mainnet endpoint. The previously used ethpandaops endpoint returned malformed null contract results/timeouts; PublicNode could not serve required old state. Do not substitute latest state, clear journals, fabricate missing results, or automatically rebroadcast through an unqualified provider. Before an endpoint switch, verify the approved chain/genesis, oldest and newest retained EIP-1898 state pins, original transaction/receipt identities, and both cold/warm history audits within the existing startup deadline. Then restart the single existing journal owner with an explicit `--ethereum-rpc`, unchanged wallets/deployment/output directory, and inspect canonical progress. Budget a dedicated archival execution service and historical Beacon access for production; public free access alone is not a production availability commitment.

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

[MessageQueue](../ethereum/src/MessageQueue.sol) uses UUPS: `_authorizeUpgrade` requires `DEFAULT_ADMIN_ROLE`, initially held by [GovernanceAdmin](../ethereum/src/GovernanceAdmin.sol). The admin-only `migrateToBeefy` uses `reinitializer(8)` and grants no new roles. A paused legacy proxy can atomically select a qualified, live BEEFY verifier and publication floor with expected-verifier and expected-maximum guards; existing custody, roots, maturity, genesis and replay state remain intact.

See the [mainnet migration candidate](../ethereum/README.md#mainnet-custody-preserving-migration-candidate) for retained-state fork evidence and the source activation, mainnet follower, full-accounting and governance gates. Authority impersonation and fixture BEEFY checkpoints are local test mechanisms, not production authorization.

A BEEFY client expires after 24 hours without an authenticated source timestamp. Expiry does not remove existing UUPS admin authority. Repair requires that actual approved authority and reviewed implementation/calldata, preserving custody, roots, maturity, replay protection and original transaction evidence. The one-time legacy migration is not an automatic expired-client replacement mechanism.

Fresh deployments use `initializeBeefy` with the same five base arguments as `initialize` and no wallet argument. It is not an initializer for an already initialized ZK proxy. Native deployments retain Foundry broadcasts under their run’s `foundry-broadcast/` directory rather than replacing the repository deployment journal. Do not reset retained campaign clocks, failed verdicts or unresolved obligations.

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

Also configure the actual governance admin/pauser, emergency-stop roles and positive bridging-payment fee. Simulate before an authorized broadcast, retain the Foundry broadcast journal, and never retry an uncertain partial deployment without reconciling its original receipts and nonce. The source placeholder `0x111…111` is not a production Safe. Do not deploy a recovery controller around it.

Keep a coherent task-owned Foundry output/cache for the token script and set `FOUNDRY_OUT` to the same `--out` directory for OpenZeppelin FFI validation. Mixing full-build and partial-script build-info can produce duplicate-contract validation failures; do not disable upgrade validation or overwrite prior approved artifacts to work around them. Retain the successful simulation and verify every original sender, nonce, calldata and CREATE address before accepting canonical finalized deployment receipts.

Once the original deployment is finalized, set `CLIENT`, `VERIFIER`, `MANAGER` and `QUEUE` to those contract addresses. Use the deployment wallet's protected JSON file, not a key in command arguments:

~~~sh
target/release/beefy-deploy tokens-manifest \
  --source-rpc "$SOURCE_RPC" --witness-rpc "$WITNESS_RPC" \
  --ethereum-rpc "$HOODI_RPC" --wallet "$DEPLOY_WALLET_JSON" \
  --anchor "$RUN/anchor.json" --token-stack "$RUN/tokens/token-stack.json" \
  --client "$CLIENT" --verifier "$VERIFIER" --queue "$QUEUE" --manager "$MANAGER" \
  --output "$RUN/deployment.json"
target/release/beefy-deploy tokens-configure \
  --source-rpc "$SOURCE_RPC" --witness-rpc "$WITNESS_RPC" \
  --expected-genesis "$SOURCE_GENESIS" --gear-suri-file "$GEAR_SURI_FILE" \
  --ethereum-rpc "$HOODI_RPC" --wallet "$DEPLOY_WALLET_JSON" \
  --deployment-manifest "$RUN/deployment.json" --token-stack "$RUN/tokens/token-stack.json"
~~~

Configuration authenticates the original deployment, programs, mappings, decimals, roles and native wrapper binding before resuming. Its journal preserves completed actions. Never override manifest identity or replace an unresolved signed operation.

### 3. Run and verify

Use distinct funded commitment and root signers, separate from deployment and other workers:

~~~sh
target/release/beefy-relay tokens-follow \
  --source-rpc "$SOURCE_RPC" --witness-rpc "$WITNESS_RPC" \
  --ethereum-rpc "$HOODI_RPC" --wallet "$FOLLOWER_WALLET" --root-wallet "$ROOT_WALLET" \
  --deployment-manifest "$RUN/deployment.json" --output-dir "$RUN/follower" \
  --client-deployment-transaction "$CLIENT_DEPLOYMENT_TRANSACTION"
~~~

Use the original client CREATE transaction hash from the retained deployment journal. This is only a locator: the follower still authenticates finalized ancestry, transaction/receipt roots, the signed CREATE address, approved creation bytecode and constructor bindings before retaining the bootstrap pin. A conflicting restart hint holds. Without this hint, discovering creation requires historical `eth_getCode` state, which a pruned public RPC may not serve.

Use an archival execution endpoint for `HOODI_RPC`: public Hoodi accepts HTTPS or WSS and preserves the requested transport; owned fixtures still require their approved loopback WebSocket endpoints. Both transports bound individual requests to 30 seconds. The CREATE locator does not remove the archive requirement: restart must reauthenticate historical client/queue state and original inclusions. A pruned-state error is HOLD, not permission to reset the journal or bypass validation. Before restarting on another same-network archive endpoint, run the same command and original directory with `--reconcile-once`; resume continuous following only after that succeeds.

Run one writer per durable follower/root journal and signer. Independent permissionless publishers may advance the same contracts; reconcile their authenticated progress without relabeling original submissions. Start the checkpoint, inbound and paid outbound workers using the commands earlier in this guide, with these same deployed program/contract identities and separate durable directories. Do not run the legacy ZK root publisher against this queue. Start application traffic only after authenticated catch-up and checkpoint readiness.

For a fresh queue, pass its original direct CREATE transaction through `--queue-deployment-transaction` (or `QUEUE_DEPLOYMENT_TRANSACTION`) to the outbound worker. The locator is not a trusted scan floor: the worker authenticates the finalized block, full transaction/receipt roots, signed CREATE address and chain/genesis identity, then replays inclusively from the queue birth block. Restart reauthenticates the saved birth; conflicting hints or already-progressed legacy journals HOLD. Only an untouched legacy rebuilding journal can adopt the authenticated birth atomically. Omit the locator to retain genesis replay; never reset an existing cursor or delete pending finality fences.

Paid inbound event readers preserve typed HTTP errors: a read-only provider 429 is recoverable, while authentication failures remain terminal. Resume the same discovery and transaction journals after an outage; this does not authorize retrying an ambiguous signed submission with new bytes.

Verify original queued messages, finalized commitment/root/delivery receipts, maturity, replay protection and exact recipient/escrow/supply effects. Native queued/mailbox value is not settlement. The SDK [examples](../js/bridge-js/README.md) preserve original intent and operation deadlines; full live tests require the independently approved proof profile and owned source, execution, Beacon and proxy fixtures. Unit mode is not a substitute. Frontend completion requires settlement, not merely a successful relay submission.

Each manual UI claim starts one 44-minute absolute deadline before wallet/archive setup and requests SDK waiting. Stale indexer availability is advisory, not a signing prerequisite; missing or malformed independently approved inbound profiles remain HOLD. Approval is immutable while authenticated preparation observations may advance only from `NotPresent`; outdated checkpoints, identity changes and missing/forked history hold. The SDK rechecks original, previous and final hash pins across Beacon I/O.

For historical outbound backfills, the selected source height/message/proof stays fixed while unsigned simulation waits only on the decoded `MerkleRootDelayNotPassed` error. Each retry reauthenticates finalized queue root and anchor and uses the same deadline; signing occurs once after maturity. Other failures are immediate. Preserve any known original Gear/Ethereum transaction hash in HOLD and reconcile it before retrying; an unresolved remote wallet request has no invented hash or cancellation guarantee. Only authenticated token/native settlement triggers UI completion.

Offline periods require catch-up and reconciliation of original signed work, not re-genesis, fresh nonces or reset deadlines. No uninterrupted timed soak is required. The follower retains completed history indefinitely, rewrites its entire JSON journal, and reauthenticates retained commitments within a 300-second startup deadline. Qualify disk growth and restart time for the intended operating period; no automatic history compaction is implemented.

Manager replacement uses an authenticated Beacon-slot cutover, not replay-key export/import. Keep the original manager and its evidence queryable; retain the same native wrapper. Prepare and drain all canonical pre-boundary receipts, original signed work, native payouts, outgoing requests and definite refunds before custody transfer. Authentic unpaid pre-boundary deposits require explicit `--sponsor-unpaid`; they never acquire fabricated fee evidence. Keep the replacement paused through source retirement, token/wrapper role handoff and finalized readiness. See the [manager procedure](../gear-programs/vft-manager/README.md).

Native lifecycle commands with a unit result use the pinned Sails 0.9.2 empty automatic success reply; nonunit replies still require their exact route and complete SCALE decoding. Discovery rejects malformed or unadmitted public Redirect/proof/reconciliation candidates without treating them as retained-evidence corruption. Only an authenticated successful or pending-native original can own an external admission: an unpaid self-relay rejected while paused does not prevent explicit sponsorship. After a role call finalizes before its reply is saved, resume the same cutover work: the tool reauthenticates and persists that original before checking successor role readback. Live role state alone never proves original success or authorizes a replacement signature; retained bytes, nonce, reply and canonical history remain strict.
External discovery infers native reconciliation ownership only for an admitted pending-native parent, and only through its first canonical successful `ReconcileReceipt` reply reporting `Processed`. Redundant public reconciliation after successful parent or native completion, including later failed calls, cannot replace that original or delay cutover. Earlier retained terminal attempts and the exact successful continuation remain authenticated alongside full historical economic settlement; malformed retained evidence and unresolved earlier originals still HOLD.
Pending external admissions from older discovery may refresh only their inferred public continuation projection after canonical rediscovery authenticates the exact unchanged parent signature, inclusion, message, reply and provenance. The saved parent discovery pin is still reauthenticated; settled admissions and locally owned signatures/attempts retain strict evidence-preservation checks. Completion requires full historical settlement at or after retained original replies; bounded row eviction or missing archive still HOLDs, and nonce order does not establish which read-only reconciliation caused completion.

Seal the exact old transaction, block, discovery and completed coverage/admission bytes in immutable `cutover.json`; install matching `.cutover-seal` markers while old writer locks remain held. Normal relayers must not reopen sealed lanes. Only read-only `verify-cutover` may reauthenticate their original evidence. Successor journals are new paths bound to that seal and the approved new receiver, beginning at execution block B+1 and receipt slot S+1. An incomplete handoff or unavailable original history is HOLD, not permission to relabel the old target.

Stop both old lane writers before preparing the cutover; the tool exclusively locks their original journals. `OLD_TRANSACTIONS` is the transaction directory (including `state.json` and `blocks.json`); `OLD_DISCOVERY` is the discovery JSON file. `NEW_MANAGER_CODE_ID` must come from independent approval of the unused, manually paused replacement. Keep the same paths, boundary and identities on every retry:

~~~sh
target/release/vft-manager-tool --gear-endpoint "$SOURCE_RPC" prepare-cutover \
  --old-vft-manager "$OLD_MANAGER" --new-vft-manager "$NEW_MANAGER" \
  --new-manager-code-id "$NEW_MANAGER_CODE_ID" \
  --old-transactions "$OLD_TRANSACTIONS" --old-discovery "$OLD_DISCOVERY" \
  --ethereum-rpc "$EL_RPC" --beacon-rpc "$BEACON_RPC" \
  --boundary-slot "$S" --cutover-dir "$CUTOVER_DIR"

target/release/vft-manager-tool --gear-endpoint "$SOURCE_RPC" finalize-cutover \
  --cutover-dir "$CUTOVER_DIR" \
  --new-transactions "$NEW_TRANSACTIONS" --new-discovery "$NEW_DISCOVERY" \
  --ethereum-rpc "$EL_RPC" --beacon-rpc "$BEACON_RPC" \
  --evm-registration-transaction "$REGISTRATION_TX"

target/release/vft-manager-tool --gear-endpoint "$SOURCE_RPC" verify-cutover \
  --seal "$CUTOVER_DIR/cutover.json" \
  --ethereum-rpc "$EL_RPC" --beacon-rpc "$BEACON_RPC"
~~~

Supply the authorized Gear lifecycle signer through protected `GEAR_SURI` for prepare/finalize; the CLI development-key default is not an operator identity. `verify-cutover` neither uses a signing credential nor changes journals. Add `--sponsor-unpaid` to prepare/finalize only with explicit authorization to pay for authentic unpaid deposits. Otherwise unresolved unpaid receipts remain HOLD; previously fulfilled external receipts require their own authenticated original and settlement evidence, never borrowed lane provenance.

Preparation leaves the source `Draining`. Finalization resumes original custody and role calls, requires the actual finalized authorized EVM successor-registration transaction while retaining the old registration, seals the historical lane, initializes the new lane at B+1/S+1, and only then activates it. The tool has no EVM signer. Missing registration is HOLD after custody/role handoff; perform that authorized registration externally and resume the same directory. Do not restore or reuse old lane files to bypass a HOLD. `DeployUpgraded` only aliases preparation to an existing paused replacement; it does not deploy or copy replay keys.

Retain `cutover-work.json`, `cutover-coverage.json`, `cutover-calls.json`, `cutover.json`, `cutover-activation.json`, original journals and matching seal markers. Start the successor with the tool-reported `--cutover-seal` and `--successor-manager-code-id`; a retry after activation authenticates the original activation and preserves successor progress.

Both indexers wait through valid finalized-head lag in 30-second polls without committing any part of the batch. Previous finalized pins and the complete batch remain authenticated; missing or conflicting history fails closed. Shut down the processor to cancel this wait. Scheduling confirmations are not finality. For an existing database, preserve the separate [event-coordinate reindex requirements](internals.md); schema migration alone does not authenticate old rows.

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

1. Verify the Gear endpoint is a dedicated, trusted node and can serve finalized blocks and GRANDPA justifications.
2. Verify the Ethereum RPC can read finalized state and the fee payer has enough native currency.
3. Verify the MessageQueue address, genesis authority-set hash/id, and verifier/SRS data belong to the same deployment.
4. Verify every persistent directory exists and is writable by the container user, while secret files are not world-readable.
5. Render the supervisor configuration and inspect every path and port.
6. Start with `RUST_LOG=relayer=info,prover=info` and confirm the relayer loads storage, initializes authority-set sync, starts its HTTP server, and exposes `/metrics`.
7. Only then enable the token relayers that consume roots and submit user-facing transfers.
