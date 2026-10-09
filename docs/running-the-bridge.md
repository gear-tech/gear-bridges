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
- Foundry v1.8.3 (forge/cast, matching CI) for the Ethereum contracts and deployment tooling.
- The native build dependencies listed in [Dockerfile](../Dockerfile), including a C compiler, OpenSSL development files, CMake, protobuf compiler, and Clang.

The root README also calls out the [ring build instructions](https://github.com/gear-tech/ring/blob/main/BUILDING.md). Follow those instructions when a native build fails while compiling `ring`.

The Go module pins the original ignition-verifier commit through a public mirror because the upstream repository is unavailable; the cryptographic implementation and revision are unchanged.

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

For a transient RPC failure, the Gear listener reconnects and replays missing finalized blocks; Ethereum pollers reconnect and resume from their persisted cursors. The shared Ethereum WebSocket handshake and individual RPC requests time out after 30 seconds for `EthApi`, `PollingEthApi` and raw-provider calls. Transient receipt-evidence transport failures reconnect and recheck the original signed transaction; identity, finality and event mismatches remain HOLD. A timeout is not proof of non-submission and does not authorize replacement signing, a different nonce or changed deadlines. A child task that exits permanently is surfaced as a process failure. Inspect logs and metrics before deciding whether to retry, restore state, or escalate to the emergency procedures.

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

Outbound completion requires canonical finalized Ethereum ancestry and the configured confirmation depth, not eight latest-head confirmations alone. A bridge-nonce watcher also checks authenticated finalized processing when the original transaction receipt never appears. Completion through another transaction retains the original UUID/hash and records no attributed original receipt. Nonce and processed-state reads use the same canonical finalized block hash. Reorganizations, RPC failures and missing transaction visibility retain the original identity; they cannot authorize a replacement nonce.

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

Canonical mining permits the next same-chain submission, but does not count as finalized completion. The follower records mined commitments separately: `lastMinedUpdate` advances on verified inclusion, while `lastSuccessfulUpdate` and `lastFinalizedUpdate` advance only after canonical finality. Pending entries retain their original nonce, signed bytes, transaction hash and source identity. An absent previously observed inclusion still holds the lane. Changed inclusion can recover only after independent finalized-header, transaction/receipt-trie, source-proof and accepted-checkpoint authentication of that same signed transaction. The complete first receipt is retained as `firstInclusion`; previously finalized history and saved root checkpoint pins cannot move. Conflicting or incomplete publication evidence holds instead of reanchoring. Within-set refreshes after 32 source blocks leave inclusion headroom for the unchanged strict 64-block readiness limit.

On follower restart, saved finalized commitments are revalidated against their original signed identity, receipt inclusion, finalized anchor and source block before any new submission. A finalized boolean is not sufficient. The worker uses one fresh verified finalized view, deduplicated targets and bounded 32-way checks. Durable hash-linked RLP headers and transaction/receipt trie evidence are locally authenticated on reload; incomplete ancestry fences and interior progress survive interruption beyond the unchanged 32,768-header memory bound. History revalidation and pending-transaction reconciliation share one 300-second startup deadline. Changed or incomplete evidence holds instead of skipping history, resetting the journal or extending warmup deadlines.

`beefy-relay tokens-history-audit --source-rpc <source> --witness-rpc <witness> --ethereum-rpc <EL> --deployment-manifest <manifest> --follower-state <existing-state.json> --proof-dir <new-private-audit-directory>` exercises that read-only historical authentication path without a signer. Keep the original journal untouched and use a separately owned proof directory, never the running follower’s archive. Repeat with the same audit directory for the warm-recovery check; fresh deployment history is not evidence for aged-journal recovery.

A root publication pins the latest canonically mined BEEFY anchor before signing. Its durable intent blocks further handovers until the original root transaction is canonically mined; finality can then proceed alongside subsequent handovers. Root status `mined` is not `accepted`: acceptance requires canonical finality of both the pinned anchor and root transaction. Restart never silently reanchors a signed publication. Use an archival execution RPC for historical checkpoint and receipt verification; a pruned-state fallback is not supported.

### BEEFY expiry and existing administration

[MessageQueue](../ethereum/src/MessageQueue.sol) already uses UUPS: `_authorizeUpgrade` requires `DEFAULT_ADMIN_ROLE`, initially held by [GovernanceAdmin](../ethereum/src/GovernanceAdmin.sol). The existing `reinitialize()` is restricted to that role and uses `reinitializer(7)`; it grants `DEFAULT_ADMIN_ROLE` and `PAUSER_ROLE` to `0x1111111111111111111111111111111111111111` without revoking the existing admin. This is upgrade and pause authority, not a new recovery contract.

The `0x111…111` address is a source placeholder: production must replace it with the approved real Safe address and verify the deployed authority and its actual owners/policy. Tests may only impersonate it with Foundry `prank`; that is not evidence of a deployed Safe or independent control. There is no bespoke 3-of-5 threshold or timelock added by the queue.

A BEEFY client expires after 24 hours without an authenticated source timestamp. Expiry does not remove the existing UUPS admin authority. Any repair must use that actual approved authority and reviewed implementation/calldata, preserving custody, roots, maturity, replay protection and original transaction evidence. This guide supplies no public migration initializer or automatic verifier-replacement workflow; public cutover remains subject to the [migration gates](zk-to-beefy-migration.md).

Fresh deployments use `initializeBeefy` with the same five base arguments as `initialize` and no wallet argument. It is not an initializer for an already initialized ZK proxy. Native deployments retain Foundry broadcasts under their run’s `foundry-broadcast/` directory rather than replacing the repository deployment journal. Do not reset retained campaign clocks, failed verdicts or unresolved obligations.

## Isolated Hoodi BEEFY token qualification

This lane is fresh-deployment-only. Do not attach it to, upgrade, or transfer custody from the public ZK queue, ERC20 manager, VFTs, token mappings, or nonce ledgers. Public migration remains blocked pending verified source identity, storage layout, liabilities and the actual approved administrative authority. The candidate BEEFY client expires after 24 hours without an authenticated source timestamp; fresh queues retain existing UUPS administration. Passing local Gear→Hoodi qualification does not authorize a public upgrade.

The corrected common queue retains the public root-keyed timestamp mapping at slot 12 and appends the BEEFY root floor at slot 14 and block-local timestamps at slot 15. Do not upgrade the retained Hoodi candidate whose slot 12 is block-keyed. New qualification needs a distinct corrected queue and journals; preserve the old lane, deadlines and unresolved obligations. Use `getMerkleRootTimestampForBlock(uint256)` for effective maturity and `getMerkleRootTimestamp(bytes32)` only for retained legacy values.

See the [BEEFY/MMR Hoodi lane architecture](internals.md#beefymmr-hoodi-lane) for source commitments, wire formats, verification boundaries and test-only deployment limits. The legacy ZK lane and its public custody remain separate.

Token preparation allocates every configured balance and allowance shard before marking a VFT active, including wrapped, Gear-origin, and native VFTs. Resuming a deterministic program repeats the same idempotent allocation operation; it does not reconstruct the program or append capacity. Metadata and initial roles must match, and the manager remains paused until preparation and mapping are complete. An old held receipt is not retried by storage initialization.

Retained fast-runtime profile only: Use a fresh raw genesis, two archive validators, independent Hoodi signers, and new deployment/relayer journals. Keep both validator RPCs on loopback. After both validators have finalized and connected to each other, run `beefy-relay tokens-source-state --raw-spec "$RAW_SPEC" --source-rpc "$SOURCE_RPC" --witness-rpc "$WITNESS_RPC" --output "$RUN/source-chain/launch-state.json"`. This one-shot read-only command checks the actual raw-spec digest, matching genesis/domain/MMR activation, current/next authorities, finalized hash, reciprocal peers, and 3000 ms/64-slot BABE configuration before marking the launch ready; it does not start or supervise validators. Pass its output to `tokens-soak --source-launch-state "$RUN/source-chain/launch-state.json" --raw-spec "$RAW_SPEC"`. Keep both files and validator databases on restart. The campaign pins immutable launch identity and rehashes the raw spec; updated readiness may reflect later finalized blocks without changing identity.

New normal-runtime qualification instead requires the exact sealed `runtimeProfile` shared by `run.json`, the bundle and verification descriptors, the source launch identity and supervisor admission. Its independently supplied `runtime-approval.json` must authorize test implementation of pinned #5642; no RPC response or retained fast-lane descriptor can create that approval. Use 3000-ms slots, 2400-slot/two-hour epochs, a distinct run/campaign and fresh corrected deployments. Runtime code hashes are labeled separately from artifact SHA256. Current-head CI, the exact release artifact and explicit execution approval remain gates; configuration files do not authorize deployment or activation.

The approved PR #5642 revision is `19ab81dd208b3ce7b339f5d1dbd144511d3c7e2c`, replacing `f961bed815dd4ab0802703620605ea3b3659ac60` only for the optional Unix-matrix CI guard. Gear runtime Rust is unchanged. New profile admission rejects the superseded pin; do not rewrite retained profiles, journals or deadlines, or transfer old artifact qualification to newly built bytes. The CI-only approval does not authorize a production-profile build or public-chain actions.

For accelerated functional checks alongside the normal lane, use the separate `fast-runtime-hoodi` profile and a `hoodi-fast-runtime-<suffix>` campaign. Start from the same approved #5642 commit and change only `EPOCH_DURATION_IN_BLOCKS` from 2400 to 64; keep 3000-ms slots, BEEFY `min_block_delta = 8`, quorum, proof verification, bridge logic and Hoodi finality unchanged. Pin the exact one-line source diff as `cadencePatchSha256`, independently select the resulting native/runtime hashes, and set `functionalOnly: true`. Do not substitute the retained #5644 fast checkout or relabel its artifacts as #5642.

The accelerated profile retains all six assets, two genuine authority handovers, a real supervised restart, 60-minute token-batch deadlines and 44-minute application-attempt deadlines; only its warmup is one hour rather than five. Use a fresh genesis, run, keys, ports, contracts, journals and separately qualified application artifacts. The sealed campaign prefix must agree with its runtime profile before setup. Normal admission rejects shortened cadence or fast-only metadata; fast admission requires the nonzero cadence-patch pin.

Accelerated results establish functional behavior on that modified test runtime, not production-cadence validation or production approval. Keep the normal lane and its original deadlines running separately. Both profiles retain `testOnly: true`, `executionAuthorized: false` and `releaseQualified: false` in artifact qualification; authorized local/Hoodi execution is recorded separately, and current CI status is not inferred from the approved base commit.

For the source-runtime upgrade and later BEEFY activation, use the [runtime upgrade and activation runbook](runtime-upgrade-and-beefy-activation.md). It separates the qualified `System.set_code` proposal from real validator-key readiness and the later atomic Root domain/activation batch. It does not authorize a build, governance submission, activation or public bridge cutover.

Shared verification on 2026-10-06 passed 195 Foundry tests, 84 BEEFY tests, the affected optimized actor suites and 82 SDK unit cases. Both full default SDK test commands failed before their live cases because the owned live fixture/profile was unavailable; explicit unit-mode skips are not replacements. Local Anvil transactions and retained-source read-only smoke checks do not qualify the normal-runtime lane, its five-hour warmup, actual authority roster, production artifacts or public migration.

For Hoodi checkpoint initialization, independently approve a recent weak-subjectivity Beacon block root and the network genesis validators root; pass them as `checkpoints-tool --trusted-bootstrap-root` and `--trusted-genesis-validators-root`. The Beacon endpoint supplies data, not the trust anchor. Use an epoch checkpoint supported by its bootstrap endpoint. The program verifies the bootstrap committee proof against the approved header, then verifies the later signed update in the same sync-committee period; the two finalized headers need not be identical.

Use the later applied checkpoint for token deployment: `checkpoints-tool` reports it as `checkpoint slot`, while `bootstrap slot` names the separate original trust header. The checkpoint program initially stores the verified update header, not the bootstrap header. For an already deployed program with mislabelled setup metadata, preserve the original record and trust pins, authenticate its initial applied checkpoint on chain, and reconcile only the slot/hash handoff; do not redeploy the program or replace the approved bootstrap root.

Checkpoint committee points must be on curve before their compressed encodings are authenticated against the committee root. Signatures receive full curve/subgroup validation before native BLS use. The signature domain uses the preceding slot at a fork boundary; committee selection still uses the signature slot itself. Gas-sensitive owner checks must use the optimized release-profile WASM from the same locked all-targets graph as deployment, with the original gas limits. Preserve debug-profile gas failures as separate evidence rather than raising those limits.

The retained genuine Hoodi fixture spans committee periods 492 → 493 → 494 with 10,614 canonical headers. The owning replay test uses one program across both rollovers, rejects each missing-following-committee update, and requires terminal Finished with exact checkpoint progress after each authenticated replay. Acquisition provenance and its original signed updates are retained; the one-hour live campaign is separate evidence.

Advancing finalized state into a new sync-committee period requires authenticated following-committee keys; a finality-only update cannot rotate committee ownership. Async verification commits only against its original state revision. Replay retains an immutable base checkpoint, excludes concurrent normal advancement, and inserts strictly increasing history. The checkpoint worker prioritizes the full committee update before newer finality-only work and treats stale-state rejection as no progress. These guarantees require the corrected freshly built WASM and regenerated client; they do not retrofit an older deployed program. Replay callers stop on `Finished`; an exhausted history batch with `InProcess` returns an error instead of advancing the local base or update watermark. Restart resumes the program’s retained replay state.

`StateChanged` remains SCALE error index 10; bootstrap, period, proof, update and header errors are appended at indices 11–15. Keep `api/gear/checkpoint_light_client.idl` and the frontend checkpoint client’s exported error union and runtime registry aligned with the Rust IO type. Cargo build scripts generate build-local metadata, not these checked-in mirrors.

Use the same `rtk cargo build --locked --release --all-targets` feature graph for token preparation, configuration, and operation. Package-only rebuilds can change embedded WASM identities. Gear's nested WASM build reads each owning program's `Cargo.lock` independently of the outer workspace lock. Retain the qualified per-program locks and verify exact optimized CodeIds after the full build; do not substitute the outer lock or accept regenerated identities. Before configuration, fund the validator stash's transaction fees and initialize the bridge: a `ForceNone` genesis needs a genuine session-key change, not merely advancing sessions. Use a separate setup key from the reserved warmup rotation key, then unpause the initialized builtin and wait for finality.

After `tokens-manifest` has pinned the fresh deployment, run `beefy-relay tokens-configure --source-rpc "$SOURCE_RPC" --witness-rpc "$WITNESS_RPC" --expected-genesis "$SOURCE_GENESIS" --gear-suri-file "$SETUP_GEAR_SURI_FILE" --ethereum-rpc "$HOODI_EL_WSS" --wallet "$RUN/keys/follower" --deployment-manifest "$RUN/deployment.json" --token-stack "$RUN/token-stack/token-stack.json"`. Use the same Gear governance/setup account used to create the fresh stack; it must not be the campaign or worker account. The development-only command verifies deployed contract and program identities, ERC20 decimals and four Ethereum-supply VFT peers plus Gear-origin GOT and native WTVARA peers, payment and governance settings; it records each completed action and resumes only an identical configuration. Leave the Gear manager paused on mismatch/interruption. Do not rerun `tokens` after handing VFT roles to the manager. This does not attach an existing public manager or migrate public custody. Both `tokens` and `tokens-configure` require a non-symlink mode-0600 setup SURI file; do not put signing secrets in command arguments.

Configuration checks the signed `anchor.sourceGenesis` and `anchor.bridgeDomain` emitted by `tokens-manifest`. Keep the manifest unchanged; do not add top-level identity overrides.

Fresh normal-lane configuration includes six assets: USDC, USDT, WBTC and WETH retain Ethereum supply; ordinary GOT and native WTVARA use Gear supply. The ordinary Ethereum fixture token grants mint authority only to its ERC20 manager. Configure the native wrapper/manager binding while paused and preserve the explicit resume action. Never admin-mint native inventory or identify native policy by token symbol.

After configuration, the profiled branch of `ops/hoodi/provision_campaign.py` invokes `tokens-provision-source-inventory` with independent source/witness RPCs, the original deployment and token stack, and separate protected setup/campaign SURI files. GOT receives 100 raw units from its setup minter. WTVARA receives two existential-deposit units through `VftNativeExchange.Mint` with real attached native value, currently 2 × 10^12 raw, leaving a gas reserve. Finalized source/witness balance, supply and backing evidence is retained in `token-stack.json.sourceInventory` and the final campaign inventory. Signed bytes, nonce and hash are persisted before submission; an unresolved original message HOLDs rather than minting again. This inventory covers the two requested preflight roundtrips, not an additional 24-hour native campaign.

Source inventory requires matching source/witness cadence: either 2400-slot normal epochs or 64-slot approved fast epochs, both with 3000-ms slots. Provision before the follower takes the deployment lock; for reconciliation, pause only the owned follower and resume it after the original inventory operation releases that lock. Sails unit-returning mint actions acknowledge with an empty payload, not a service/method envelope. Preserve that original acknowledgement and verify the finalized balance, supply and native backing; do not replace a successful mint because its acknowledgement is empty.

Run `beefy-relay tokens-follow --help` and `beefy-relay tokens-soak --help` for the actor and campaign arguments. `tokens-soak` requires `--raw-spec`, the manifests, endpoints, wallets/signers, `--follower-dir`, `--inbound-dir`, `--outbound-dir`, and campaign output directory. Those directories must be the running actors’ durable directories; canonical paths are pinned for resume.
Use a new, empty campaign output directory for schema 3; older campaign journals are rejected without migration. Keep the same directories and accounts for `--mode preflight`, `--mode warmup`, `--mode start`, and `--mode resume`. Before preflight, warmup, and T0, readiness requires a healthy caught-up follower, empty inbound and outbound backlogs, canonical worker cursors through finalized heads, and a non-replaying checkpoint through the finalized Beacon header. Missing worker files mean not ready; malformed journals or canonical hash disagreement fail closed. Readiness and key rotation precede the warmup clock. Automatic inbound readiness additionally binds schema-6 runtime identity (with explicit schema-5 compatibility) to the exact source genesis, Ethereum chain, contracts/programs and finalized manager-creation block, with a nonzero worker signer distinct from campaign and governance accounts; manual-operation journals are not automatic-worker evidence.

The outbound discovery/event journal remains schema 2; original-transaction completion is schema 3, with active schema-2 transactions read explicitly and unproven historical completion held. The inbound worker writes schema 6 and explicitly reads schemas 5/6: original signed proof/request/reply identity remains immutable, and a native `ReconcileReceipt` continuation has separate signed bytes, account nonce, request/reply and effect pin under the same inbound owner. Restarts drain original signed calls by nonce before admitting a continuation. Retained schema-4 inbound journals are preserved and rejected, never rewritten into a claim of completion.

Retained fast-runtime profile: Preflight requires two canonically finalized four-token roundtrip batches within 60 minutes each for newly admitted batches. Earlier 44-minute batches retain their saved deadlines and verdicts. Resuming an in-flight batch uses its saved balance baseline and original absolute deadline, including readiness waits; fresh-only zero-balance checks cannot reject the batch's own locked or minted assets. Completed governance evidence is retained and the pause probe is not repeated. Each governance handoff is journaled before submission; interruption holds before connection or another governance call, requiring reconciliation of the original intent rather than automatic unpause. Warmup samples on 60 fixed minute slots and still runs the full hour after readiness. Work and restart catch-up cannot shift those slots or backfill missed samples. Every minute checks worker/checkpoint readiness, including the authoritative outbound active/failed delivery index; empty discovery queues alone are insufficient. Completion audits all five accounting quantities across all four tokens against the original baseline within its saved terminal 60-second deadline. Wall-clock rollback holds before any resume save or readiness work; saved batch, warmup and terminal deadlines are never extended.

Warmup must demonstrate two genuine authority handovers and a real midpoint restart through the OS service supervisor. A readiness marker is not a restart: retain changed process/startup identities and authenticated catch-up evidence within the original fixed sampling window.

The distinct normal profile uses a five-hour warmup and requires two authenticated ordinary-cadence handovers plus actual supervisor restart/catch-up. Its normal and priority batches retain the admitted deadlines and original four 1-raw-unit fixtures, adding a 1-raw-unit GOT roundtrip and a native roundtrip of one actual existential deposit (currently 10^12 raw). Gear-origin assets begin on Gear, are escrowed and minted on Ethereum, then return; native return burns manager escrow and pays native value directly. Queued/mailbox value is not settled. Completion requires the original delivered payout and finalized manager settlement, never user-owned WVARA as a substitute. An original `NativePending` receipt requires journaled non-economic `ReconcileReceipt` unless it is already processed with every log settled. Existing 60/44-minute admissions, failed verdicts and retained clocks are not rewritten.

For SDK/application admission, provide the independently approved `inbound-proof-profile.json` in the selected application artifact directory; list its digest in the artifact manifest and qualify that manifest digest. It binds the network/runtime, source pin, actor CodeIds, exact IDLs, fork schedule, endpoint framing and original consumer route. Full default SDK gates require owned source/EL/Beacon/proxy fixtures plus `INBOUND_PROOF_PROFILE_PATH`; CI writes the latter from `INBOUND_PROOF_PROFILE_JSON` and rejects a missing secret. Unit-only mode is not live qualification. SDK callers provide the exact consumer decoder/effect verifier and original outbound expected effect. Frontend `onFinalized` requires settlement, not `RelayResult.ok` or proxy `Relayed`; missing configuration stays HOLD before signing.

The checkpoint and Ethereum-event SDK query clients retain their explicitly supplied actor IDs, like the historical-proxy client. Construction does not start background program-storage watchers or follow actor inheritance; profile validation remains responsible for authenticating the pinned actors. Explicit checkpoint event subscriptions still require their returned unsubscribe function.

GitHub-hosted live CI can reach a local archive node through `tools/beefy-relay/ops/readonly-rpc-gateway.mjs`, run from the dependency-installed checkout (not the standalone sealed Python-ops bundle). First run `node tools/beefy-relay/ops/test_readonly_rpc_gateway.mjs`. Generate a fresh owner-only token file without printing it:

~~~sh
TOKEN_FILE="$BEEFY_RUN/rpc-gateway.token"
node -e "require('node:fs').writeFileSync(process.argv[1], require('node:crypto').randomBytes(32).toString('hex')+'\\n', {mode:0o600,flag:'wx'})" "$TOKEN_FILE"
node tools/beefy-relay/ops/readonly-rpc-gateway.mjs \
  --upstream ws://127.0.0.1:9972 --port 19601 --token-file "$TOKEN_FILE"
~~~

Choose the archive RPC from the intended run, not an unrelated retained deployment. Terminate HTTPS/WSS at an operator-approved tunnel or reverse proxy targeting **only** `http://127.0.0.1:19601`. The client URL is `wss://<approved-host>/rpc/<token-file-content>`; keep the entire URL in the protected `VARA_WS_RPC` secret, never a repository file, command-line argument, PR comment or access log. Unauthenticated WebSocket upgrades fail before connecting to the node; ordinary HTTP is not forwarded. Do not expose raw development RPC: `--rpc-methods Safe` still permits extrinsic submission.

The gateway admits only the SDK's explicit metadata, pinned storage/header, bridge-proof, reply-simulation and subscription methods. Arbitrary `state_call`, transaction/admin methods, batches, malformed requests and foreign subscription cancellation are rejected. Requests are capped at 1 MiB, responses at 16 MiB (full runtime code must fit), with at most 16 clients, 32 in-flight requests and 16 subscriptions per client, and a 30-second request timeout. It is not a general relayer RPC gateway. Stop the process and rotate the token when retiring the fixture; a temporary tunnel hostname is not durable CI infrastructure. A reachable endpoint does not replace independent profile approval, full live SDK checks or actual transfer/replay/restart evidence.

For new normal/fast profiled runs, seal component/runtime qualification first; actor IDs and owned SDK fixtures do not exist yet. After deployment, collect the full ping build, handler build/tests, SDK typecheck, both default live SDK test files and compiled-example build in immutable run-owned qualification logs. An independently selected application report binds the original run/campaign/runtime profile, `componentBundleSha256`, `componentVerificationSha256`, every command/exit/log digest and the application manifest. Admit it once with `setup-services.py applications --qualification "$REPORT" --qualification-sha256 "$REPORT_SHA256"`; this creates immutable `qualification/<campaign>-applications.json` without changing the sealed bundle, starting transactions or resetting any campaign deadline. Both the wrapper and compiled client reject missing/failed checks, mutable or changed logs, foreign profiles and artifact-owner changes. Retained unprofiled lanes keep their original sealed application selection.

Readiness and warmup observations wait when an authenticated commitment receipt has a different inclusion from an unfinalized follower record. Only the follower may recover that inclusion; the observer never rewrites it or admits the changed receipt before recovery. Re-observation and RPC waits share the original readiness or minute-slot deadline. A changed finalized inclusion, invalid receipt or source authentication failure still fails closed. Failed preflight verdicts remain immutable; this wait does not authorize retrying a failed campaign.

Signer/identity admission accepts the follower's normal transient statuses `starting`, `healthy`, `catching-up`, `held-root` and `held-finality`. Readiness still requires a healthy caught-up follower, authenticated finality and the original lag/deadline checks. Failed or unknown status, local-rehearsal mode and a foreign deployment remain rejected.

Commitment submission/resume, accepted-commitment verification and pending-finality replay wait for an unavailable header at the original receipt block number, then require its original canonical hash. Header waits are bounded to 120 seconds; submission/resume share the existing 120-second original-transaction deadline rather than starting a new budget. A changed canonical hash still fails immediately. No replacement transaction, journal reset or supervisor PID-fence relaxation is permitted.

For outbound token completion, match the configured queue's original message hash and nonce to the retained ERC20 manager, not the campaign beneficiary. A later authenticated queue root may cover that same message. Record the actual delivery root separately from the original message block and check its 300-second maturity under the unchanged deadline. The canonical successful finalized receipt, processed bit, authenticated burn beneficiary and finalized user/escrow/supply balances remain required.

Each batch authenticates its distinct queued roots concurrently, with at most four read-only waits sharing the original absolute deadline. Each completed root is journaled immediately; an unresolved root cannot erase already-saved evidence. Source/witness agreement, coherent follower authentication, original signed publication identity, canonical finality and maturity checks are unchanged. Overlapping RPC reads does not shorten Hoodi finality or turn a late completion into a passing batch.

The outbound accumulator retains up to 100 roots per authority and 100 authority sets. A newer immature root cannot hide an older mature covering root. Persisted roots remain a plain JSON array and reload through the same bounded admission path. Evicting a root does not mark a message stuck while another retained root covers it; the queue's role-specific maturity delays remain enforced.


The warmup rotation saves its original signed extrinsic and handoff marker before submission. An ambiguous handoff permits read-only reconciliation of that exact identity, not a fresh signature or another rotation; both source nodes must agree on its finalized inclusion. Pre-sampling recovery retains its original readiness deadline, and sampling cannot restart after it has begun. Dependent ERC20 locks require the original canonical successful mined approval and a later account nonce; approval finality is audited with lock finality rather than adding a separate finality wait. The original approval inclusion is never overwritten.

The normal batch requires two live USDC receipt rejections after the independent mint and saved afterMint checkpoint, before any VFT approval or burn. Through HistoricalProxy/SubmitReceipt, changing only the saved proof’s transaction index to u64::MAX must produce EthereumEventClient(InvalidReceiptProof); replaying the original processed receipt must return the same receipt and nested AlreadyProcessed. Each successful verdict retains the campaign signer’s original message/enqueue identity, canonical finalized proxy reply, decoded rejection, unchanged five-quantity accounting across all four assets, and the receipt’s Processed status. Native gas is separate. Both probes share that batch’s original 44-minute deadline; missing or ambiguous replies do not pass or trigger an automatic resend. Old passed or burned normal batches without this pre-burn evidence hold, and warmup requires both probe records.

That 44-minute probe budget belongs to the retained profile. Normal admission preserves the separately approved campaign's original deadline and runtime-profile digest; neither renaming a profile nor restarting a command extends an admitted deadline.

On an interrupted preflight, readiness permits only unpaid queued messages reconciled against that batch's original finalized source intents and exact nonce, message hash and block identity. Unrelated work, paid-pair backlogs and every other readiness gate remain blocking. Resuming after burn revalidates earlier balance checkpoints at their original canonical block hashes; current post-return balances cannot overwrite the saved mint evidence or trigger another VFT approval for an existing burn intent.

For bounded test-only validation, stop after `preflight` and `warmup`; do not invoke `start` or `resume`. Passing those modes does not establish 24-hour qualification or independently controlled recovery authority.

Before reusing any passed campaign window, the observer revalidates its pinned historical balances, original lock/release inclusions, finality anchors and finalized Gear event hashes. Successful observations retain their first receipt and timestamp evidence; changed history holds without rewriting a prior result.

Receipt coverage waits for Beacon/EL finality and the finalized Gear checkpoint covering the receipt slot. A newer checkpoint not yet present waits within the original deadline; an outdated checkpoint or canonicality error fails immediately. Source and worker recovery does not pause qualification clocks or turn a failed attempt into a passing one.

Each asset retains submission handoff, EVM block/finality, Beacon coverage, worker proof composition/handoff, finalized Gear status, mint, burn, fee, root registration/maturity, and finalized release milestones. The first Gear reply or error remains separate from later status queries; its absence after a crash is not fabricated. Timestamps ending in `ObservedAtMs` are observer wall-clock times, not chain inclusion times. Root registration and eligibility come from the source-block-keyed contract timestamp and 300-second delay. Completed assets are saved independently; an unrelated pending asset cannot erase their evidence. Qualification rejects an asset without the required worker handoff and operation milestones.
The effective block timestamp is read through `getMerkleRootTimestampForBlock(uint256)`, not the historical root-keyed getter. A registered post-floor root without its block timestamp is an error, not an already-mature repeated hash.
Retained fast-runtime 24-hour mode: Only `start` creates T0. Preserve the original journal for interrupted or ambiguous operations; never delete it or choose a new T0 to bypass a missed hourly deadline. Each of the 24 windows requires all four tokens, 1 raw unit per token, with normal and priority paid returns alternating by hour. A late or incomplete window fails qualification.

Start `tokens-follow` under an OS supervisor as an independent service before qualification. This actor alone advances the BEEFY follower and publishes token roots; the campaign is only a stimulus and observer. For example:

~~~sh
beefy-relay tokens-follow \
  --wallet "$RUN/keys/follower" \
  --root-wallet "$RUN/keys/root-publisher" \
  --deployment-manifest "$RUN/deployment.json" \
  --output-dir "$RUN/follower"
~~~

`--wallet` identifies the follower signer and `--root-wallet` the root-publisher signer. Keep those EVM wallets/keys distinct from one another and from the campaign wallet. The actor uses the immutable deployment manifest and holds its deployment-scoped owner lock beside that manifest, keyed by chain ID and queue. The campaign does not acquire the writer lock, start/stop the actor, or write its artifacts.

Keep the actor directory durable and private. Schema 3 records `mode: hoodi-token-follow`, the exact deployment manifest, `followerSigner`, `rootPublisherSigner`, `localRehearsal`, service `follower.status` (`starting`, `healthy`, or `failed`), `startupSequence`, `rootScan: { block, blockHash }`, `roots`, `activeEthereum`, and client-keyed `recoveryBootstraps`. Recovery candidate records and superseded candidate history remain in this same journal. Production uses `localRehearsal: false`; the campaign rejects state with it set true.
The finalized scanner observes `QueueMerkleRootChanged` (emitted when messages change the queue), saves each exact `{ block, blockHash, queueId, queueRoot }` registration before advancing `rootScan`, and stores it under `<block>-<root hex without 0x>`. Each root retains `{ block, blockHash, queueId, queueRoot, status: pending|mined|accepted, publication: <path> }`; keep registrations needed by queued messages even after the live queue clears. Mark a root accepted only after its exact canonical finalized on-chain registration is reconciled. Root discovery continues between incoming BEEFY commitments while waiting for a handover; a pending root requests a strictly later checkpoint without waiting for an authority change.
After the queue has accepted its first nonzero root, independently witnessed initialized-zero MMR proofs can advance its source cursor across idle periods without `QueueMerkleRootChanged`. Catch-up drains steps of at most 57,600 blocks under a strictly later accepted MMR anchor, deferring real roots until they fit the next window and never crossing a retained pending real root. Empty progress creates no root or acceptance timestamp and does not restart maturity. It cannot initialize an empty destination queue; the first real root does that.
Each publication path is `$follower_dir/root-publications/<block>-<roothex>.json` and contains schema-3 evidence, including `publicationReceipt: { block, blockHash }` or `reconciledAt` at a canonical finalized head; root evidence also records `localRehearsal`. The signed transaction bytes, original nonce, and hash are durable before broadcast; recovery can only rebroadcast those identical bytes or reconcile that hash. Receipt authentication uses the accepted checkpoint’s latest MMR leaf, not an older queue registration’s timestamp.
Commitment records carry `clientAddress`, their authenticated canonical inclusion block/hash, and an explicit `finalized` flag. On recovered reinclusion, `firstInclusion` preserves the complete originally observed receipt while the current inclusion and finality fields bind the newly authenticated canonical history. Untagged legacy records belong only to the original client and cannot bootstrap a replacement. Mined records permit same-chain sequencing, not finalized completion. The actor also refreshes an idle live client at a 12-hour authenticated-source-age threshold measured against Ethereum block time.
Follower commitment submissions also reserve their nonce before signing and persist `rawTransaction` and `txHash` before broadcast. A signing or gas-estimation failure resumes the same reserved nonce; an uncertain send resumes only the saved signed bytes. A legacy pending commitment without either a hash or a reserved nonce remains held for operator reconciliation. Never infer permission to resend from an empty transaction pool or reset the journal to force progress.
With the supervisor stopped, `tokens-follow --reconcile-once` finishes existing pending commitment and root finality, even without a pending checkpoint submission. It retains the original publication nonce, signed bytes, hash, and accepted anchor; it does not discover new roots or schedule new empty progress. A registered root without a covering checkpoint remains held until the normal follower advances that checkpoint.

`tokens-manifest --local-rehearsal` and `tokens-follow --local-rehearsal` are test-only, default false, and only permit the loopback `ws://` endpoint used by a local Anvil Hoodi fork. The manifest and actor must agree on this immutable mode. Chain ID, Hoodi genesis, contract policy/domain, and proof checks remain mandatory. Production qualification requires `wss://` and rejects rehearsal evidence.

When the campaign records `restartGate`, the operator must perform a real restart through the OS supervisor. The campaign verifies a higher persisted `startupSequence`, recovered state, and catch-up; it cannot pass by simulating that restart.

`tokens-publish-root` remains a serialized one-shot maintenance command, not a second running publisher. Stop the actor through its supervisor, run maintenance against the same manifest/owner lock, then restart the actor. Never run maintenance concurrently with the actor. Maintenance must use the unique original follower registration and its exact publication path under that actor's `root-publications` directory; alternate-path intents are rejected.

Every failed preflight or campaign verdict stays failed, even if no token action or T0 was created. Never reuse, copy, or hard-link the actor directory or locks from a failed campaign; never reset T0 or replay a held/ambiguous receipt to create a passing run. Preserve the failed report and journals; reconcile held receipts separately without resubmitting them. Never discard a pending BEEFY submission.

An execution-layer finalized receipt can precede the Beacon endpoint’s finalized execution payload. The controller waits for the Beacon payload to cover the canonical receipt without extending the batch's saved absolute deadline. Both RPC calls and polling sleeps share that deadline; late coverage, a canonical mismatch, or a missed deadline still fails preflight. Inbound receipt reconciliation reads the VFT manager at a finalized Gear block with a block-gas-limited Sails query; a failed query holds the original receipt key for investigation rather than resubmitting it.

The source RPCs are development-key signing endpoints: never point them at a remote chain or publish them beyond loopback. Keep the publisher, follower, campaign, paid relayer, and deployer EVM keys separate and fund each for its role. Clients with one through three validators authenticate the full native quorum in both submission protocols; the two-validator profile verifies both signatures. This is not independent-host security or one-Byzantine-validator resilience. Larger sets retain the existing sampling policy. A passed local Anvil BEEFY rehearsal does not establish Hoodi inbound delivery; use the actual Hoodi execution and beacon endpoints for live preflight and canonical finality. Inspect the campaign verdict and all 96 per-asset records before asserting success. Queue maturity is keyed by source block, not root hash: publishing the same root in a later block does not reset an earlier block’s delay. The timestamp API takes a block number. Deploy a fresh queue; this mapping-key change is not an upgrade or migration of a protected existing queue.

### Sealed local deployment operations

The checked-in entry points are in `tools/beefy-relay/ops/`. They target macOS launchd, Python 3.11+, the reviewed native Gear executable, real Hoodi, and the two-validator loopback profile only; this is not a one-command portable deployment. A logged-in macOS GUI session is required for the `gui/<uid>` launchd domain. Keep the host on AC power and preserve the generated caffeinate job during timed observations. Linux/systemd and independent-host validator deployment are not implemented by these scripts.

Install `tools/beefy-relay/ops/requirements.txt` in a new isolated Python environment and set `OPS_PY` to its absolute Python executable; do not modify an older run's environment or use optimized Python (`-O`/`PYTHONOPTIMIZE`). Also provide `rtk`, Foundry `forge`/`cast`, and Node on PATH: the Python requirements do not install those binaries, and the offline ops check uses Node. The selected Gear, BEEFY relay, relayer and checkpoint tool come from the qualified bundle, not PATH. The running actor plists use `$HOME/.foundry/bin`, `$HOME/.cargo/bin`, `/opt/homebrew/bin` and system directories, so `rtk` must also be available there. Do not relocate or remove the Python environment while its plists are installed.

~~~sh
python3 -m venv "$OPS_VENV"
OPS_PY="$OPS_VENV/bin/python"
"$OPS_PY" -m pip install -r tools/beefy-relay/ops/requirements.txt
"$OPS_PY" -m pip check
"$OPS_PY" tools/beefy-relay/ops/test_ops.py
~~~

Set `OPS_VENV` to a new absolute directory. `RUNS_DIR` must already exist as a private operator-owned directory; `BUNDLE` must name a new output directory under an existing parent. The funding wallet is a nonsymlink mode-0600 JSON file with matching `address` and `private_key`; never put its contents in commands, logs or a published run descriptor. Each prepared UUID owns its keys, genesis, ports, contracts and journals. Preparation probes but does not reserve ports; choose unused `--source-rpc-port`, `--source-p2p-port`, `--source-metrics-port` pairs and a four-port `--service-port` range when another run exists. Never stop or borrow an older run to make those ports available.

Build and qualify the selected artifacts first. `seal-artifacts.py` does not run tests or confer qualification: it requires a schema-1 verification JSON with `status: VERIFIED`, successful `cargo-tests`, `forge-tests`, `full-release-build`, and `historical-recovery` checks. Each check records `name`, actual `command`, `exitCode`, `logPath`, and `logSha256`. The record also contains `binaries` (`gear`, `beefy-relay`, `relayer`, `checkpoints-tool`, each with `path` and `sha256`), `sourceFiles` (path-to-SHA256 map), and `solidityArtifactSha256`. Retain the original logs and fingerprint all build inputs; a manually asserted status is not test evidence.

`sourceFiles` must cover the copied operations tree, Foundry configuration files and every copied Solidity `out/` file, including build-info and dynamically loaded artifacts. Sealing checks destination copies against those original qualification digests and originally parsed verification bytes before publishing `bundle.json`. A concurrent rebuild or edit must fail sealing, not become a newly blessed bundle hash. Keep any failed output for diagnosis; it is not deployable without a completed manifest.

Fresh simulation, checking and broadcast also require the local Foundry configuration (including qualified file absence), sole build-info and all runtime-loaded contract artifacts to match the sealed bundle. This includes the dynamically loaded WrappedVara implementation. Those hashes are part of the simulation and deployment-intent identity; an older simulation needs a fresh dry-run. Drift after a durable intent holds that original attempt for reconciliation, not another send.

~~~sh
"$OPS_PY" tools/beefy-relay/ops/seal-artifacts.py \
  --verification "$VERIFICATION_JSON" --ethereum-project "$SOLIDITY_PROJECT" \
  --output "$BUNDLE" \
  --runtime-approval "$RUNTIME_APPROVAL_JSON" \
  --runtime-approval-sha256 "$RUNTIME_APPROVAL_SHA256"
"$OPS_PY" "$BUNDLE/ops/prepare-run.py" \
  --runs-dir "$RUNS_DIR" --funding-wallet "$FUNDING_WALLET_FILE" \
  --funding-address "$FUNDING_ADDRESS" \
  --runtime-profile normal-runtime-hoodi \
  --execution-http "$HOODI_EL_HTTP" --execution-wss "$HOODI_EL_WSS" \
  --beacon-http "$HOODI_BEACON_HTTP" \
  --trusted-bootstrap-root "$TRUSTED_BOOTSTRAP_ROOT" \
  --trusted-genesis-validators-root "$TRUSTED_GENESIS_VALIDATORS_ROOT"
~~~

Use the new run path printed by `prepare-run.py` as `BEEFY_RUN`. Preparation verifies artifact hashes and HTTP/WebSocket/Beacon identity, allocates fresh role keys, and copies the sealed Solidity project; it does not fund or deploy. Include the complete qualified `cache/solidity-files-cache.json` in verification `sourceFiles` and in the bundle alongside its exact artifacts. Deployment binds the cache hash and `FOUNDRY_EXTRA_OUTPUT_FILES=[]` into simulation and intent: use the sealed IR sidecars without regenerating them. Otherwise Foundry's auxiliary-file checks force recompilation and produce partial, duplicate build-info; relocating a full rebuild also changes absolute paths in build-info. Do not normalize metadata or relax hash guards. Execute subsequent scripts from **that sealed bundle's** `ops` directory, not the mutable checkout. `run.json` pins the bundle digest, endpoints, source/service ports and funding identity. Runtime entry points reject changed artifacts, non-loopback source RPCs, public/symlinked private-key files and optimized Python. Do not edit a sealed bundle or transplant journals into another run.

Supply a recent independently approved weak-subjectivity checkpoint and the independently approved Hoodi genesis validators root. Do not copy the bootstrap root from the deployment provider as a substitute for approval. Preparation records both pins before deployment; the checkpoint intent and deployment retain them on resume. A changed or missing pin holds the operation without redeploying or rewriting the original intent.

For this normal-runtime command, verification must include `campaignName: hoodi-normal-runtime-<suffix>` and the exact `runtimeProfile` validated by `setup-services.py`: approved #5642 commit `19ab81dd208b3ce7b339f5d1dbd144511d3c7e2c`, 3000-ms slots, 2400-block epochs, five-hour warmup, two handovers, 60-minute token batches and 44-minute application attempts. Include the independently selected `gearBinarySha256`, runtime code SHA256/Blake2b256/Keccak256, `approvalSha256`, and explicit `runtimeCiStatus`. The independent approval JSON has `status: APPROVED_FOR_TEST_IMPLEMENTATION` and `runtimeProfile` equal to that profile with only `approvalSha256` omitted; the separately approved raw-file SHA256 must match both `RUNTIME_APPROVAL_SHA256` and the profile pin. This artifact selection is not execution approval: retain `testOnly: true`, `executionAuthorized: false` and `releaseQualified: false`. Record separate operator authorization before funding, source governance, deployment or campaign spending; unresolved or failed CI is not production qualification.

Declare dependency aliases explicitly in the root Foundry configuration: the sealed source snapshot does not carry nested dependency configuration used by automatic remapping discovery. Compile an isolated copy and compare every qualified output before selecting a corrected bundle. If a packaging defect is found after funding, preserve the original bundle, descriptor, roles, source state, salts and journals; a bundle-selection change requires an explicitly approved, recorded transition, not an in-place edit or disabled hash guard.

The ordered setup commands are below. Each step must finish with its own canonical evidence before continuing; a process launch or a marker alone is not readiness. Funding and deployment commands spend test currency and are only for an authorized fresh deployment. The pinned Gear executable requires `key generate-node-key --chain dev`; its inspection commands do not take that selector. Keep a failed setup's original funded roles, genesis and any generated peer keys; do not rerun identity reservation or genesis generation.

`setup-funding.py prepare` creates private local role keys and funding journals without sending transactions. Its no-argument mode funds five role EOAs (0.05 deployer, 0.10 follower, 0.025 each root/paid/campaign: 0.225 Hoodi ETH total, plus gas) and waits for canonical finality, retaining at least 0.5 ETH in the funding wallet after maximum recorded costs. It does not deploy a Safe. Source initialization/governance, program preparation/configuration, token deployment, inventory provisioning, bootstrap queueing and running actors also sign or spend test funds/native value. A dry run and `--check` do not broadcast, but read the protected deployer key; default token-deployment mode broadcasts. Never treat a printed setup plan, funding balance, runtime approval JSON or guide command as authorization, and never point these commands at a public Vara source or mainnet.

~~~sh
export BEEFY_RUN="$NEW_RUN"
OPS="$BUNDLE/ops"
"$OPS_PY" "$OPS/setup-funding.py" prepare
"$OPS_PY" "$OPS/setup-funding.py"
"$OPS_PY" "$OPS/source-chain/pin-source-identity.py"
"$OPS_PY" "$OPS/source-chain/prepare-source-genesis.py"
"$OPS_PY" "$OPS/source-chain/setup-source.py" start
"$OPS_PY" "$OPS/source-chain/initialize-bridge.py" --stage prepare-keys
~~~

For this fresh normal profile, genesis preparation explicitly sets `Beefy.GenesisBlock=None` and checks the native raw-storage result; the embedded runtime stays unchanged. The native local preset otherwise starts BEEFY at block 1, which cannot be treated as a later governed activation. Wait for the original finalized key registration to enter the real session and for `GearEthBridge.BridgeInitialized`; do not shorten epochs or reset the chain to accelerate this wait. Only then continue:

~~~sh
"$OPS_PY" "$OPS/source-chain/initialize-bridge.py" --stage activate-beefy --activation-delay-blocks 1
"$OPS_PY" "$OPS/source-chain/initialize-bridge.py" --stage unpause
"$OPS_PY" "$OPS/source-chain/setup-source.py" ready
"$OPS_PY" "$OPS/setup-programs.py" prepare
"$OPS_PY" "$OPS/setup-programs.py" anchor
"$OPS_PY" "$OPS/prepare-token-deployment.py" --dry-run
"$OPS_PY" "$OPS/prepare-token-deployment.py" --check
"$OPS_PY" "$OPS/prepare-token-deployment.py"
"$OPS_PY" "$OPS/finalize-token-deployment.py"
"$OPS_PY" "$OPS/setup-programs.py" configure
"$OPS_PY" "$OPS/hoodi/provision_campaign.py"
"$OPS_PY" "$OPS/setup-services.py" all
"$OPS_PY" "$OPS/bootstrap-queue.py" queue
# After the original source message, root and worker delivery finalize:
"$OPS_PY" "$OPS/bootstrap-queue.py" record
~~~

Provide `ETHERSCAN_API_KEY` through the environment for deployment verification, never through command arguments. The dry run and broadcast use the same pinned constructor inputs and artifacts. The wrapper redacts secrets, preserves the child-held deployment lock, and writes the deployment intent before broadcast. An existing deployment intent prohibits automatic rerun: reconcile the original Foundry transaction hashes, nonces, receipts and CREATE addresses instead. Source setup likewise saves original signed extrinsics before handoff and only reconciles or rebroadcasts those exact bytes.

Bootstrap queues the original nonce-0 same-value governance-pauser update, not an asset transfer or a synthetic root. Its marker identifies the original source extrinsic, signed paid-worker UUID/transaction, accepted anchor, root publication, and independently obtained before/after balances. Recording requires canonical finality and no active/failed outbound deliveries; campaign readiness independently authenticates the evidence. Never manufacture a bootstrap marker, mark a delivery completed by hand, or reset the source nonce.

Bootstrap indexing requires the current schema-3 completed outbound transaction journal; the schema-2 discovery/event journal is a separate format. Original signed transaction, canonical finalized receipt, source inclusion and unchanged economic-accounting checks remain mandatory. Funded GOT/WTVARA source inventory is not a bridge liability: bootstrap permits its recorded Gear balance/supply while requiring zero Ethereum wrapped supply and bridge escrows across all six assets.

Sails unit acknowledgements (including governance pause/unpause and bridge-fee payment) have an empty payload, not a service/method route envelope. Non-unit replies still require the exact route and complete SCALE decoding. A client decoding failure does not undo a finalized governance pause: retain the original signed action and failed campaign, reconcile its on-chain effect, and obtain explicit operator authorization before unpausing or starting a separately labeled campaign; never reset the failed journal or its deadlines.

`beefy-relay tokens-snapshot` is read-only and requires explicit source/witness, Ethereum/Beacon endpoints, deployment/token-stack manifests, public Gear/EVM user identities, and an output path. It records pinned cross-chain quantities without signer inputs. It does not establish campaign readiness, ledger completeness, or permission to retire an older lane.

Local role keys do not establish approved production administrative authority. Keep `publicMigration: BLOCKED` and `productionQualification: NOT ESTABLISHED`. Preserve older services, keys, journals, pending identities and failed reports until their liabilities and control transactions have been reconciled separately.

After fresh setup and bootstrap recording, `setup-services.py all` has written, but not loaded, campaign-bound one-shot plists. The continuously supervised actors have `KeepAlive=true`; preflight and warmup have `KeepAlive=false` and `RunAtLoad=true`, so explicit bootstrap starts each once without restart-on-exit. Only after explicit authorization and actor readiness, load the selected preflight once, then the supervised warmup observer after a terminal passed preflight. Already bound older plists remain immutable; do not patch their flags or admission digests in place:

~~~sh
CAMPAIGN_NAME="$("$OPS_PY" -c 'import json,sys;print(json.load(open(sys.argv[1]))["campaignName"])' "$BUNDLE/verification.json")"
/bin/launchctl bootstrap "gui/$(id -u)" "$BEEFY_RUN/supervisors/$CAMPAIGN_NAME-preflight.plist"
# Wait for terminal passed preflight; do not restart a failed or held attempt.
/bin/launchctl bootstrap "gui/$(id -u)" "$BEEFY_RUN/supervisors/$CAMPAIGN_NAME-warmup.plist"
~~~

Do not kickstart these one-shots, launch warmup separately, or call `tokens-soak start/resume` as a substitute. The observer performs the real midpoint follower restart under the original deadline and deployment-wide campaign ownership. Application admission remains a separate post-deployment qualification step. These commands are not instructions to transition or reprovision a retained lane. Functional normal/fast six-asset roundtrips have passed, but retained timed campaigns failed; they do not establish timed or production qualification.

### Retained Hoodi milestone admission

Do not re-run provisioning for the retained `f991e59c-71cb-4b0a-8c6c-56ce2acb8ac1` lane. Preserve both the original `campaign/` and terminal failed `campaigns/hoodi-milestone-1/` histories. The next separately qualified campaign is `hoodi-milestone-1-retry-1`. The runner, observer and one-shot plists use the hash-authenticated selected `verification.json.campaignName`; only `hoodi-milestone-1` or `hoodi-milestone-1-retry-N` with positive, unpadded N is valid. The requested campaign directory must not exist at transition admission. Existing journal identity, original finalized balances and deployment-wide signer ownership must pass before credentials are read.

Seal the reviewed same-lane continuation candidate. Gear, the runtime, economic/checkpoint actors, deployed Solidity artifacts, economic WASMs and ABIs remain unchanged; only reviewed follower/campaign-observation corrections may change `beefy-relay`. The reviewed ops slice includes explicit qualified campaign propagation in the warmup observer. Both bundle roots and their complete inventories must be read-only. The verification input must include all nine exact commands below and the existing sealer checks named `cargo-tests`, `forge-tests`, `full-release-build` and `historical-recovery`. Every check needs exit code zero and a hash-pinned log; source-file hashes and all four binary records must match the candidate.

~~~sh
python3 tools/beefy-relay/ops/test_ops.py
cargo nextest run -p beefy-relay
cargo build --locked -p ping --release
forge build --root js/bridge-js/js-test/contracts --force --no-cache
forge test --root js/bridge-js/js-test/contracts --match-contract MessageHandlerTest -vvv
yarn workspace @gear-js/bridge typecheck
yarn workspace @gear-js/bridge test test/vara-to-eth.test.ts
yarn workspace @gear-js/bridge test test/eth-to-vara.test.ts
yarn workspace @gear-js/bridge build:examples
~~~

Also require `embeddedPrograms` with VERIFIED status and each deployed program's name/path/codeId/sha256, and `originalIntentReconciliation` with VERIFIED status, runId, the original unnamed campaign-tree SHA-256, an empty unresolvedOriginals list, and a hash-pinned JSON evidence path containing `finalizedAssets` matching the fresh four-asset snapshot. Retain the original failed verdicts, nonce/transaction identities and absolute deadlines.

That same reconciliation document must retain the unnamed evidence and contain `namedCampaigns: {oldName: {status: "VERIFIED", campaignTreeSha256, unresolvedOriginals: [], ...finalizedOriginalEvidence}}`. Its keys must exactly match the existing `campaigns/` children. Each child must be a private nonsymlink schema-3 journal with matching runId, terminal `failed_before_t0` and failed preflight, and its entire tree must match the recorded digest. Changed, unrecorded, nonterminal or unresolved history HOLDs admission. Existing app intents or deployment still prohibit another campaign transition. Historical and named one-shot jobs must already be unloaded; this admission check never boots them out.


Use the exact candidate path and digest printed by `seal-artifacts.py`:

~~~sh
export BEEFY_RUN="$RUNS_DIR/f991e59c-71cb-4b0a-8c6c-56ce2acb8ac1"
# OPS_PY must name the retained lane's existing isolated Python environment.
CAMPAIGN_NAME="$("$OPS_PY" -c 'import json,sys;print(json.load(open(sys.argv[1]))["campaignName"])' "$CANDIDATE/verification.json")"
# For the retained recovery-5 predecessor, use a new recovery-6 journal:
TRANSITION_NAME=hoodi-milestone-1-recovery-6
"$OPS_PY" "$CANDIDATE/ops/setup-services.py" transition --candidate-bundle "$CANDIDATE" --candidate-sha256 "$CANDIDATE_SHA" --transition-name "$TRANSITION_NAME"
# Only after the recorded transition reaches terminal PASS:
/bin/launchctl bootstrap gui/$(id -u) "$BEEFY_RUN/supervisors/$CAMPAIGN_NAME-preflight.plist"
# Only after the native preflight reaches terminal passed:
/bin/launchctl bootstrap gui/$(id -u) "$BEEFY_RUN/supervisors/$CAMPAIGN_NAME-warmup.plist"
~~~

The transition records original file/process identities, stops and fences the six original jobs, preserves private worker snapshots, and changes only the recorded bundle/component/plist bindings. Acquire the bundle-transition, program-setup, service-setup, source-supervisor, deployment and bounded-campaign locks in that order. After job absence and wrapper/native process exit are proven, acquire the original deployment root-owner lock and `follower/state.lock`. Unfinished native save files remain frozen in the snapshot and HOLD the transition for their existing owner; never remove them to continue.

Every successor retains the predecessor's complete mutable-file inventory. Old campaign plists keep their exact bytes through old-to-same digest pins; a newly qualified campaign adds only its two absent one-shot plist paths with old=None. The current binding remains `supervisors/hoodi-milestone-1-observer-binding.json`. Freeze reconciled old campaign trees alongside all immutable application trees in `preservedArtifactTrees`. Existing historical transition authentication and partial-apply checks still apply.


The stop and source gates retain their original 30-second and 120-second deadlines. `--resume` requires the same candidate/name and accepts only recorded old or intended new file digests. A third digest or an expired gate stays HOLD/FAILED. Final observations must finish inside the original deadlines. A loaded but unrecorded job, reused PID/start/group identity, changed supervisor binding or missing previously dispatched actor is HOLD, not permission to bootstrap a replacement.

Startup now pins the dispatched PID and start time, waits within the original gate for that process to exec the exact sealed native binary, and requires its own process group before authentication. launchd can initially report group 1 before assigning the group to the PID; only that initial transition is allowed. A Python wrapper is not readiness, even when its command matches the plist. Interrupted saves preserve the observed identity; later PID, start-time or group changes still HOLD.

Keep `source-chain/launch-state.json` and the original campaign unchanged. Authenticate the raw `source-before.json` and `source-after.json` digests recorded in `transition.json`. The new source identity must match the old launch identity with only `relayBinarySha256` changed. Source and witness must retain genesis/domain/A/S/runtime, the anchor and pre-stop finalized hashes, and nonregressing common finalized history; their current/next authority sets must remain valid and consecutive. Advancing finalized heights and natural session changes are allowed. These two local observations are not organizationally independent validation.

The replacement `qualification/components.json` must have `status=VERIFIED`, the selected `bundleSha256`, the candidate's measured `checks` and `verificationSha256`; admission requires its recorded intended-new file digest. Preserve `hoodi/network-gate.json` and `supervisors/service-ports.json`; service endpoints must match the retained descriptor. Final PASS also requires current identities for all six actors, applied checkpoint and commitment/root advancement, finalized history authentication and unchanged per-asset balances. Recheck every retained identity/history digest and applied binding before PASS. Bootstrapped jobs or PIDs alone do not pass. The transition never launches a campaign.

Both new one-shots have `KeepAlive=false`. The supervised observer starts the selected runner's warmup with the same inherited campaign lock and performs at most one follower restart inside the native minute-30 gate. Do not kickstart either one-shot or start warmup separately. Standalone observer invocation cannot qualify the supervised restart.

After both native phases and the supervised restart proof pass, use the selected sealed wrapper for applications:

~~~sh
BUNDLE="$("$OPS_PY" -c 'import json,os;from pathlib import Path;print(json.loads((Path(os.environ["BEEFY_RUN"])/"run.json").read_text())["bundle"]["path"])')"
OPS="$BUNDLE/ops"
CAMPAIGN_NAME="$("$OPS_PY" -c 'import json,sys;print(json.load(open(sys.argv[1]))["campaignName"])' "$BUNDLE/verification.json")"
"$OPS_PY" "$OPS/run-preflight.py" app --campaign-name "$CAMPAIGN_NAME" --direction eth-to-vara -- --mode=verify --intent-id=inbound-relay-1
~~~

Application admission runs only the matching compiled example under the artifact directory authenticated by the selected bundle’s verification record, using that directory’s packaged Node, never a PATH-selected Node. Seal both example entrypoints and the complete transitive dependency inventory. Package Node with mode 0500, remaining files with mode 0400 and directories with mode 0500; reject symlinks, special files, writable entries, missing files and changed hashes.

The selected bundle's `verification.json` must contain `applicationArtifacts: {path, manifestSha256}`. Use a canonical nonsymlink immutable directory within the retained RUN's `app-messages/`; preserve existing artifact trees. `manifestSha256` hashes the final raw manifest, which requires `schemaVersion=1`, `testOnly=true`, the retained `runId`, the selected qualified `campaignName` and the complete `files` hash map. A separately campaign-bound tree is allowed only after both old and candidate trees pass full `app_artifact` authentication and their manifest objects, excluding only `campaignName`, are exactly equal. Code, file inventory, run identity, testOnly and other metadata cannot change. The manifest is not its own trust anchor; the selected sealed qualification pins its path and digest before any service changes or client execution.

The native token phases execute the selected `beefy-relay` directly, without a mutable `rtk` hop. Both token and app children retain the inherited deployment-wide campaign lock until they exit, including when their Python wrapper dies. Write modes pass `ETH_CAMPAIGN_KEY_FILE` and `GEAR_CAMPAIGN_SURI_FILE` paths for the retained idle campaign credentials, never secret argument values. Verify mode does not access either credential. No implicit deploy/send mode or fallback to mutable examples is allowed.

Qualification change notes: admission now pins component bytes and the external application manifest/runtime; resume preserves original dispatch identities and deadlines; final transition status requires current actor and applied-state evidence. These are reviewed candidate changes, not a completed supervised transition, live token/message qualification or mainnet approval.

The 2026-10-03 retained-lane observer transition exhausted its original source deadline after wrapper-to-native startup HOLDs. Its source gate remains FAILED and its transition remains HOLD. Both original source nodes started; checkpoint, inbound, outbound and follower remained fenced, and no named token campaign or application deployment started. Preserve its journal SHA-256 `f9df51d41637a5772b45a0c780871ca40a7e68ba2dfe2621633be7382e82a9d1` and all of its evidence. The startup correction passed offline ops checks and an isolated launchd exec smoke; those checks do not change the failed verdict.

#### Repeatable, separately journaled Hoodi continuation

The current user authorizes routine fixes, restarts, separately journaled Hoodi attempts and test-ETH distribution. No permission round or code patch is required for each later attempt. Mainnet signing and activation remain prohibited. The recorded authorization is `review-evidence/hoodi-continuation-1791108724029/authorization.json`, SHA-256 `a547601d263afb7e405997430fe1364274bdf13a2d0fd91da312c5328a0f0047`, under the retained build run. The helper does not distribute funds; the operator retains original funding intents, canonical finalized receipts and separate native-fee accounting.

The failed observer journal remains SHA-256 `f9df51d41637a5772b45a0c780871ca40a7e68ba2dfe2621633be7382e82a9d1`. Recovery-1 remains SHA-256 `b375c46f6f0c006b704ae2159983e9d30a89e547622872bdbfac594afe93137d`: its stop/source gates passed, but its applied-progress deadline failed while the follower was unfunded. It is not a PASS after a refill. Its selected bundle is `b06ec093d307e841ed6652d02d2e96c9f73037a35680318dd7ee844af75e85f8`; its source and downstream actors remain supervised. `app-messages/public-recovery-1-result.json` is retained result history, not evidence of a started application.

Transition names retain the logical `hoodi-milestone-1` recovery lineage independently of token campaign names. Use the next unoccupied `hoodi-milestone-1-recovery-N` after the pinned installed predecessor, authenticating any intervening expired unbound preparations. An unresolved nonterminal HOLD must be reconciled through its existing owner, not superseded. A new transition gets its own `bundle-transitions/<name>/` directory and frozen gates. Existing directories require `--resume` with their exact name/candidate. Failed gates and every previous journal/tree remain unchanged; no success search, rollback, history reset, replacement signed intent or automatic campaign launch is permitted.

Supply these additional bindings in the candidate verification input, alongside the measured checks and retained application/program/reconciliation evidence above:

- `continuationAuthorization: {path, sha256}`: the canonical authorization JSON, with its exact raw-file hash and Hoodi-only/preserved-history scope.
- `continuationPredecessor: {path, sha256}`: the transition selected by the current `hoodi-milestone-1-observer-binding.json`, not an older passing journal. For recovery-2 this is the recovery-1 record and hash above. Its bundle, immutable frozen bindings, replacement snapshots, source evidence, private worker snapshot, ancestor and current applied file digests must authenticate. The helper freezes the complete inventory and digests of all historical transition trees and public result files before the stop intent. Source/worker state may naturally advance while the services run.
- `continuationFunding: {minimumBalanceWei: {follower, root, paid, campaign}}`: positive decimal-string finalized working reserves. Current selected reserves are respectively `150000000000000000`, `10000000000000000`, `5000000000000000`, and `20000000000000000` wei. These are admission reserves, not a guaranteed total-spend estimate. Before any stop/gate, authenticate actor addresses, finalized balances, original follower signed transaction hash/chain/sender/destination/nonce and maximum cost, and consumed-nonce conflicts. Idle root/paid/campaign actors must have no unsettled nonce. Funding shortages HOLD before preparing the new attempt; finalize any authorized refill first.
- When the native follower changes, `followerCatchupFix: {previousSha256, candidateSha256, sourceFiles: {path: sha256}, checks: [name]}`: exact old/new binary hashes, reviewed source hashes also in qualified `sourceFiles`, and the fresh measured `beefy-nextest`, `full-release-build` and `historical-recovery` check names. The relayer's `cargo-tests` check cannot qualify a changed follower.
- A recovery that changes the native relayer requires `relayerSchedulingFix` with the same old/new hash and qualified-source bindings, plus fresh `cargo-tests`, `full-release-build` and `historical-recovery` checks. The response-wakeup correction preserves journal formats, single-signer ownership and original transaction identities. Ordinary observer transitions still reject a changed relayer. Gear and checkpoint binaries, runtime, deployed economic programs and ABIs remain unchanged; neither native review replaces the stop, source or progress gates.

The follower fix drains eligible authenticated pending commitments before fetching more source history. Previously the restored scan already contained 1,024 pending justifications; fetching the next block hit the same bounded cap before the original nonce-861 transaction could reconcile. The fix preserves that bound, canonical cursors, handovers and signed intent. Funding alone cannot fix this ordering error.

After qualifying and sealing, use only the exact candidate path/SHA printed by the sealer:

~~~sh
"$OPS_PY" "$CANDIDATE/ops/setup-services.py" transition --candidate-bundle "$CANDIDATE" --candidate-sha256 "$CANDIDATE_SHA" --transition-name "$TRANSITION_NAME"
# Resume only this attempt, with the same arguments and --resume.
~~~

A later attempt can reuse the same immutable qualified candidate when its files and qualification still match; add `--predecessor-sha256 <current-bound-journal-SHA256>` to pin the journal named by the current one-shot binding instead of the candidate’s older `continuationPredecessor`. No editable predecessor path is accepted. Resume uses the frozen original predecessor and rejects a conflicting supplied SHA. A fresh sealed path is not required merely to restart the same reviewed actors. The attempt's binding still names its own journal. Never alter the selected core bundle or existing `artifacts-recovery-1` tree.

A successor may pass an expired preparation only when it is HOLD, remains unbound, and has no completed stop or later gate. Admission authenticates its frozen identity, original deadlines, unapplied descriptors and complete transition history, including older non-campaign transitions. The only allowed descriptor additions are the authenticated campaign's two one-shot plists with old=None; either partially installed path still HOLDs. The installed transition remains the predecessor; the failed preparation is preserved, not retried or rewritten.

Continuation reuses the fixed lock order, six-job fencing, root/state ownership locks, private snapshots, CAS and original source paths/keys above. It authenticates current native PIDs/start times/process groups rather than old baseline PIDs; an unowned native process or wrapper HOLDs. Frozen worker snapshots are taken only after fencing. Preserve incomplete native saves for their existing owner. Source, BEEFY, queue and checkpoint history must not regress, but naturally advancing finalized heights/session sets do not have to equal an old observation.

Fresh recovery preparations authenticate the complete retained history and artifact trees before freezing their gate clocks. The stop barrier remains 30 seconds; resumed attempts authenticate the full frozen record without resetting any deadline. An expired, unapplied preparation requires the separately named successor path above.

Funding admission authenticates every follower nonce between finalized and pending, including mined original transactions already moved from the active submission into commitments. Missing originals, conflicting ownership or substituted transactions cause HOLD. This check never signs, funds or rebroadcasts.

Each newly created continuation freezes its 30-second stop barrier, 120-second source gate within the 150-second preparation cap, and 120-minute applied-progress deadline before service changes. `recovery-authorization.json` is the read-only per-attempt binding snapshot, not another permission request. Resume authenticates dispatched actors without restarting them again and never widens these deadlines. Actor-start failures are journaled by the same applied-progress failure boundary: HOLD before expiry, FAILED after expiry, with original PIDs and deadlines retained. An expired readiness gate does not imply that already-started native services have stopped. A new failed gate remains an honest separate failure; diagnose the measured cause before another attempt.

The gate clocks are set once after read-only preparation and before durable authorization or service changes. Resume never resets them. Run the transition owner through an explicit launchd plist with RunAtLoad=true and KeepAlive=false. Do not use launchctl submit: it retried a failed owner automatically on this host. Preserve a failed attempt and authenticate any successor separately.

Each transition audit populates its own `history-audit-proofs` directory; never share the live follower's exclusive finality archive. A bound attempt with passed stop/source gates and an explicit progress HOLD reason may be superseded under the same admission locks. Pin and preserve its journal and original deadlines; do not relabel it PASS. Pending or running attempts remain ineligible, and campaigns still require the successor's terminal PASS.

The original 20,609-block backlog (source 63,507 versus client 42,898) required roughly 322 mandatory handovers. The initial 32-to-64-minute inclusion estimate was too optimistic: live Hoodi catch-up advanced roughly one handover per minute, and recovery-2 exceeded its frozen 120-minute gate while native services continued advancing. Preserve that failed attempt. Allow the authenticated, funded follower to catch up before starting a new separately journaled bounded acceptance attempt; never widen the expired gate or skip a handover. Token and application 44-minute deadlines and the 60-minute warmup remain unchanged. Require finalized checkpoint and BEEFY advancement, a healthy follower whose finalized commitment and root scanner reach the frozen common finalized target, then either a genuinely newer finalized message root or verified unchanged/nonregressing idle-root continuity. Authenticate the target/cursor on both sources, every retained original root/publication, signed transaction hash, sender/nonce/calldata, canonical finalized successful receipt, configured queue event and same-pin stored root. Extra/unfinalized/orphaned publications, pending root-publisher nonce or changed original evidence HOLD. `tokens-history-audit` authenticates saved commitment/receipt history; the separate hash-pinned `queue-root-continuity.json` authenticates root publications. `bridge-services.json` distinguishes idle continuity from actual new-root progress.

Pending-commitment authentication uses at most 16 concurrent read tasks and applies journal changes in original order. Signature, canonical inclusion, reinclusion and finality checks remain required; faster RPC reads do not shorten Hoodi finality. For a signer-free replay check, set `BEEFY_TEST_SOURCE_RPC`, `BEEFY_TEST_ETHEREUM_RPC` and `BEEFY_TEST_FINALITY_STATE` to authenticated archive endpoints and a private follower-state snapshot, then run `cargo nextest run -p beefy-relay -E 'test(live_finality_replay_preserves_original_transactions)' --run-ignored only --no-capture`. The check writes only a temporary journal and does not qualify token or application delivery.

Only the current bound transition's terminal PASS and all three PASS gates admit the one-shots for its qualified campaignName. Terminal campaign journals cannot be retried. Do not start token preflight while funding or native catch-up is unresolved. Each distinct campaign keeps its eight normal/priority roundtrips, originally recorded batch deadlines (60 minutes for newly admitted batches; prior 44-minute windows stay unchanged), full 60-minute warmup, exactly-once supervised midpoint restart, token balances and application delivery/replay gates. Mainnet remains independently gated. Startup idle roots, a refill, a mined transaction or a running PID cannot satisfy those criteria.

## First-start checklist

Before allowing the service to submit transactions:

1. Verify the Gear endpoint is a dedicated, trusted node and can serve finalized blocks and GRANDPA justifications.
2. Verify the Ethereum RPC can read finalized state and the fee payer has enough native currency.
3. Verify the MessageQueue address, genesis authority-set hash/id, and verifier/SRS data belong to the same deployment.
4. Verify every persistent directory exists and is writable by the container user, while secret files are not world-readable.
5. Render the supervisor configuration and inspect every path and port.
6. Start with `RUST_LOG=relayer=info,prover=info` and confirm the relayer loads storage, initializes authority-set sync, starts its HTTP server, and exposes `/metrics`.
7. Only then enable the token relayers that consume roots and submit user-facing transfers.
