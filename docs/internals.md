# Bridge internals

This page describes the current runtime composition of the bridge. The implementation is split into asynchronous services connected by channels; most services persist a cursor or work item before handing it to the next stage.

## Shared off-chain mechanisms

- `ethereum-client::inclusion` owns execution-block/receipt authentication and retained transaction/receipt inclusion proofs. Queue creation and BEEFY callers keep their own identity checks, validation order and archive ownership.
- `ethereum-client::signed_transaction` checks signed-envelope hashes, complete decoding, nonce/destination, chain and signer constraints. Each lane still validates its typed calldata and owns signing, persistence and broadcast recovery.
- `gear-rpc-client::observation` shares finalized-range extrinsic discovery and reply-to scanning. Callers qualify the range and retained pins, authenticate dispatch/reply identities, and persist their own cursors and outcomes. Deployment keeps its full-range inclusion uniqueness check.
- `gear-common::{durable, signer_ownership}` owns durable immutable writes and per-genesis/account signer exclusion, including cutover ownership. Deployment tooling no longer depends on the relayer actor for those mechanisms.
  Recovery of an identical immutable record syncs its parent directory even when `.new` is absent: a prior attempt may have stopped after unlinking `.new` but before that sync. Failure to sync remains an error, not acknowledged durable ownership.
- The SDK owns the standard VFT-manager settlement adapter and shared Gear reply traversal; the frontend supplies generated pinned readers. Subscription handling, original-request authentication and arbitrary consumer effect policies remain explicit.

These are shared evidence mechanisms, not a shared recovery state machine. Journal schemas, save-before-broadcast boundaries, native refund-versus-delivery semantics, and independent lane ownership remain unchanged.

## End-to-end map

### BEEFY/MMR outbound lane

The source implementation uses BEEFY/MMR for outbound root authentication and retains the separate inbound Beacon path. Existing public deployments, custody and actor identities are not migrated by a source change; the retained Hoodi lane and qualification limits below remain distinct from public cutover approval.

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

The native runtime used by the retained lane configures `pallet_mmr` with `Keccak256`, `MmrLeaf` and `DepositBeefyDigest`; `pallet_beefy` uses `MmrLeaf` for validator updates and ancestry. The runtime's `bridge_leaf::VaraBridgeProvider` commits the parent timestamp and queue state before delayed queue clearing. BABE authors blocks and GRANDPA finalizes the source chain; BEEFY authenticates MMR commitments for the bridge. Runtime installation and activation remain subject to the [existing qualification gates](runtime-upgrade-and-beefy-activation.md).

[`QueueSnapshot`](../tools/beefy-relay/src/protocol.rs) and the runtime provider encode exactly `version=2:u8 | "vara":4 | bridgeDomain:32 | timestamp:u64LE | initialized:u8 | queueId:u64LE | root:32`: 86 bytes. The outer MMR leaf is version 0 and 113 SCALE bytes. The inherited Solidity `parachainHeadsRoot` field in [`BeefyClient`](../ethereum/src/beefy/BeefyClient.sol) contains the snapshot hash, not parachain heads.

The domain is Keccak256 of `vara/gear-eth-bridge-domain/v2`, `sourceDomain32`, the destination chain ID as 32-byte big-endian, and `queueAddress20`, concatenated in that order. Source `BridgeDomain` configures one lane, not a destination registry. Deployment and relay evidence separately pin genesis identity.

For queue source block B, insertion L=B+1, MMR start S and commitment C, the leaf index is L-S and the leaf count is C-S+1; B<C and B>=S are required. [`Source::proof`](../tools/beefy-relay/src/source.rs) reads historical state and checks the initialization boundary, parent identity, insertion-time next authorities, leaf coordinates and signed root. Reuse it rather than reconstructing a snapshot from current state. Archive state and offchain MMR nodes are required.

Rust preflights bound SCALE lengths before decoding: at most 256 authorities, 64 payload entries, 64 KiB of payload bytes, 82,688 signed-proof bytes, one 113-byte MMR leaf and 256 MMR proof items. Hex and justification byte arrays are bounded before byte allocation; the existing WebSocket transport limits responses to 10 MiB before JSON decoding. Accepted SCALE encodings must consume their entire input canonically.

#### Verification, publication and accounting

The [EVM relay](../tools/beefy-relay/src/ethereum.rs) calls `createFiatShamirFinalBitfield` and `submitFiatShamir`. The client also retains interactive verification. Bootstrap trusts the configured current/next authority sets. Each update authenticates the newest leaf's parent timestamp; a next-set commitment hands over authority using that leaf's next-set proof. [`VaraQueueRootVerifier`](../ethereum/src/VaraQueueRootVerifier.sol) binds queue proofs to the client's latest accepted MMR anchor and the destination-bound snapshot.

External adoption and retained-original validation accept both `submitFiatShamir` and interactive `submitFinal`, selected by their exact ABI selector. Both require canonical calldata and the original source commitment, newest-leaf snapshot and next-authority binding. Adoption preserves the accepted signed bytes; it never converts an interactive transaction into a new submission.

`MessageQueue.submitMerkleRoot(uint256,bytes32,bytes)` passes the original height, root and canonical v2 envelope to `IVerifier.safeVerifyProof(uint256 sourceBlock, bytes32 merkleRoot, bytes proof)`. The verifier rejects heights above `uint32`, wrong queue/chain/domain, uninitialized snapshots, noncanonical encodings and stale or mismatched anchors; malformed ABI decoding returns false. Invalid root proofs revert with `InvalidQueueRootProof`. There is no field-element packing or selectable proof-system fallback. See [proof authentication](circuits-and-proofs.md).

A canonically mined commitment is capacity/progress evidence, not finalized acceptance. The [follower](../tools/beefy-relay/src/follower.rs) and [root publisher](../tools/beefy-relay/src/root_publication.rs) retain original source, client, anchor, nonce, raw bytes, transaction hash and inclusion. A signed root publication cannot be reanchored; finality promotion may add evidence but cannot replace that identity. [`MessageQueue`](../ethereum/src/MessageQueue.sol) retains maturity, message inclusion, global nonce replay protection, custody dispatch and historical-root redemption. Empty initialized queue progress creates no root and does not restart maturity. Completion requires canonical finalized commitment, root and release receipt evidence.

A never-mined signed commitment may resume by broadcasting its original bytes. Once an inclusion is recorded, disappearance with an unconsumed nonce is HOLD, not a broadcast trigger; retain the original inclusion until authenticated recovery qualifies it. A finalized consumed-nonce disposition can archive ownership without claiming successful delivery.

Retained finalized root recovery uses the original signed transaction, receipt/transaction inclusion proofs and finality archive through both the full publisher and its waiter. The exact client-bound commitment entry supplies the original anchor evidence, even after the client advances. Once source/witness, manifest, queue and signer bindings revalidate, recovery returns the original's accepted outcome without rediscovering receipt or transaction bodies. Corrupt retained evidence fails closed; external progress and consumed-nonce reconciliation cannot complete that original.

The corrected common queue preserves legacy slots 0–13: block→root at 11, root→timestamp at 12 and nonce→processed at 13. Appended slot 14 holds the BEEFY root floor and slot 15 holds block→timestamp; there is no controller storage slot. `getMerkleRootTimestamp(bytes32)` remains the historical API; `getMerkleRootTimestampForBlock(uint256)` and redemption share the effective timestamp reader. Per-block time takes precedence; legacy fallback is permitted only before activation or below the floor. Missing applicable timestamps fail closed. Repeated hashes mature independently; authenticated conflicts clear only the affected block. The 300/3600-second delays and historical redemption remain intact.

The immutable `beefyRootMinimum` is the admission floor, not the first publication. `genesisBlock()` records the first nonzero publication and never moves; `maxBlockNumber()` records monotonic progress. Authenticated nonzero roots may backfill any missing height at or above the floor, including below genesis or behind empty progress. A zero BEEFY floor rejects new publication; it does not disable historical redemption. Forward publications remain bounded by max plus MAX_BLOCK_DISTANCE. Empty progress requires initialization and a strictly greater height, creates no root and grants no maturity. Each backfilled block starts its own destination registration delay, even if the same root hash is already mature elsewhere.

Follower/root schema 4 records own versus external accepted transaction provenance. An external finalized checkpoint or root can satisfy unsigned progress, but never changes the identity or outcome of a signed original. Historical root inventory begins at the immutable floor and retains unpublished epochs behind the frontier. Distance catch-up publishes the actual initialized source/witness snapshot, including nonzero intermediate roots. Outbound discovery separately persists qualified inclusion hashes and transaction/log coordinates with one atomic cursor/root state; legacy numeric caches require a complete finalized replay before unsigned work resumes.

At every scanned height, both source endpoints must agree on the presence **or absence** of `QueueMerkleRootChanged` before snapshot retention or cursor advancement. A disagreement or query failure leaves that height unconsumed, including at the floor; matching block hashes alone do not authenticate an event response.

An authenticated competing root can release independent BEEFY following while the root publisher retains its unresolved signed original. The follower revalidates that observed progress before leaving `held-root`; it does not replace the original bytes, nonce or anchor, and later reconciles the original receipt separately. An unsigned standalone publication finishes on either its own accepted publication or authenticated external observation.

The follower skips same-set commitments while there is no uncovered queue-root demand. Source block distance alone is not a paid-update trigger: the old 32-block refresh was an offchain scheduling policy, not a Solidity admission rule. Every authority handover remains mandatory, with ordered source scanning and current/next-set authentication unchanged. Idle maintenance submits the next suitable commitment once the authenticated source timestamp is 12 hours behind the destination clock, targeting a 12-hour margin before the unchanged 24-hour expiry; destination clock checks are rate-limited to once per 60 seconds while ordered commitments arrive, not a guarantee when source commitments stall. Stalled source finality, missing proofs, expiry and unresolved original nonce ownership still HOLD rather than permit replacement or stale admission.

Traffic demand is the latest unpublished source root not already covered by the accepted checkpoint. The scheduler freezes this discovered batch so later traffic cannot move its target forever. One near-head commitment covers the whole batch, not one commitment per message; each historical root still needs its own authenticated publication and per-block maturity. Already-covered pending/mined publications do not trigger another same-set commitment. Busy latency is ordered catch-up plus the next native BEEFY commitment strictly after the newest root in the batch, destination inclusion/finality and the queue delay (300/3600 seconds); no fixed wall-clock latency is promised by a source-block lag count. Idle cadence is authority-set changes plus the 12-hour heartbeat when necessary, not every 32 blocks. Operational traffic-readiness lag checks remain separate from idle timestamp freshness and must not be silently bypassed.

Native queue roots are cumulative within one QueueId, but `QueueMerkleRootChanged` is emitted once at the end of every source block that appends messages, not once per authority epoch. The current publisher retains and publishes each such historical root; only messages appended within the same source block naturally share a root publication. Sharing a BEEFY commitment therefore does not imply shared root-publication fees. Artificially delaying the current publisher would add latency without eliminating its retained publications. A later root cannot stand in for an older root across a queue reset, and neither QueueId nor a processed key alone authenticates message inclusion. Budget forecasts must count changed source blocks, commitment demand batches and individual executions separately.

The owned normal-runtime fixture has 3-second slots and 2,400-slot authority epochs: idle following needs about 360 handovers in 30 days when sets change each epoch, with the heartbeat normally subsumed. Its observed same-set commitments are eight blocks (24 seconds) apart. Continuous new root demand can consequently trigger a new demand batch at that cadence; this scheduler has no gas-price cap or mandatory traffic batching delay. These are fixture observations, not a production latency guarantee or an unlimited-traffic monthly cost guarantee.

Root discovery polls authenticated finalized ranges when HTTP transport reports `PubsubUnavailable`; subscription rejection and malformed responses are not polling fallbacks. A fresh queue deployment waits until the confirmation-qualified finalized height reaches its authenticated birth block before writing any journal state, then replays that birth block inclusively. Existing conflicting journals still HOLD.

The [checkpoint worker](../relayer/src/ethereum_checkpoints/) updates the [checkpoint-light-client](../gear-programs/checkpoint-light-client/) independently. The [inbound worker](../relayer/src/message_relayer/eth_to_gear/) supplies finalized receipt/Beacon proofs through [HistoricalProxy](../gear-programs/historical-proxy/app/src/service.rs) and the [Electra verifier](../gear-programs/eth-events-electra/) to the [VFT manager](../gear-programs/vft-manager/app/src/services/submit_receipt/mod.rs). This path remains separate from BEEFY. The [paid outbound worker](../relayer/src/message_relayer/gear_to_eth/) redeems verified messages for ERC20 release.

Each durable follower/root journal has one supervised owner. Proof submission remains permissionless: independently funded publishers use distinct EVM signers and authenticated external progress is recorded separately from each original transaction. `beefy-relay tokens-follow` owns commitment following and publication; `tokens-publish-root` is the standalone publisher. Audit/status use `tokens-history-audit`, `bridge-status` and `bridge-events`. Give every independently submitting process, including retained message delivery and the observer, a distinct EVM fee payer; never share a publisher's nonce account. Keep Gear setup/governance accounts separate from checkpoint and inbound workers. Deployment code and embedded program WASMs belong to the opt-in `beefy-deploy` target; the normal follower has no campaign supervisor or application deployment dependency.

The follower journal retains completed commitments, transactions and accepted roots indefinitely and rewrites the complete JSON on save. Startup reauthenticates retained history under one 300-second deadline. Pending-queue bounds do not bound lifetime disk use or restart work; qualify history size and archive availability for the intended operating period. Never bypass authentication or delete original evidence to make startup pass.

One process may own distinct bootstrap, follower and root-proof archives for the same Ethereum genesis. Owners are keyed by canonical directory and keep their exclusive locks; path aliases reuse that owner. Every attached archive retains its own witnessed finality fence. Fresh finalized ancestry must satisfy the union of those fences before any is completed, so attaching another directory cannot discard an older pin or bypass a conflict.

#### Retained remote development lane

The retained v2.1.0 lane runs under `/srv/vara-bridge` on `bridge@62.238.54.143`, with two original validators on the same host. This is not host-level high availability or a production-security deployment. The temporary origin is `https://bridge-62-238-54-143.sslip.io/?network=testnet`. Keep whole-origin HTTP Basic authentication: the preserved chain still has publicly known development privileges, and Safe RPC does not prevent signed Sudo calls.

User units are `bridge-hoodi-{source,witness,postgres,capture,checkpoint,follower,inbound,outbound,indexer-gear,indexer-eth,api,faucet}.service`. On this host, administer them with `sudo systemctl --user --machine=bridge@.host …`; lingering provides boot persistence. Native validators use the original RocksDB bases, keystores and network identities. Rootless application containers retain the original `/Users/ukintvs/.local/share/vara-beefy-hoodi/runs/9119ee40-005f-49d9-a9f0-6142e6a3eb4e` namespace through bind mounts. Do not rewrite signed publication paths, signer ownership files, journals, or interrupted `.new`/`.tmp`/`.bak` files.

Start both validators and PostgreSQL first. Confirm the pinned genesis/runtime, common finalized history and advancing finality before enabling bridge writers. Start the capture proxy before the checkpoint updater. `/srv/vara-bridge/config/checkpoint-guard` authenticates the last captured original, canonical inclusion, typed successful reply and matching source/witness state before each updater start. Missing or unresolved evidence fails closed; it never broadcasts a replacement. The updater retains `Restart=no`: reconcile the original manually after failure, preserve its bytes and nonce, then explicitly start the unit. A captured finalized status alone is not application success.

Before continuous follower startup after a coordinated move or restart, stop its unit and run `/srv/vara-bridge/config/run-service follower --reconcile-once` exclusively. The migration also runs cold and warm `tokens-history-audit` passes with `/srv/vara-bridge/audit-cache`; retain its authentication checks and 300-second history deadline. Resume the original paid workers, indexers and API only after reconciliation. Readiness compares canonical cursors against configured safe heads: Gear's 10-block and Ethereum's 75-block scheduling buffers remain distinct from authenticated finality.

Ethereum deposits appear in indexed history only after the 75-block ingestion buffer and authenticated finality, typically around 15 minutes after inclusion. The visible history page refreshes its transaction list every 60 seconds even when empty or when every listed transfer is complete, so newly indexed deposits are discovered without a manual reload.

For Gear settlements, `completedAtTxHash` identifies the event's executing extrinsic, not necessarily the signed message-enqueue extrinsic. An unsigned `Gear.run` hash can repeat across blocks: verify it with `completedAtBlock`, the canonical block hash and the event's extrinsic phase, and verify the signed enqueue original separately. Hash reuse alone is not a stale indexer record.

For a guarded stack restart, use `/srv/vara-bridge/config/guarded-restart.py`. It first publishes authenticated maintenance and stops the faucet, then stops the checkpoint updater while capture and both nodes remain available and runs its original-outcome guard. It stops the follower, paid workers, indexers and API, then capture, PostgreSQL and both validators. It restarts in the preceding order, reconciles before writers resume, and starts the faucet last. Maintenance remains until independent checks prove advancing chain/checkpoint/cursors plus unchanged original settlement; only then restore live routes. Never reboot the shared host just to exercise this sequence.

Unit output is available through `sudo journalctl _SYSTEMD_USER_UNIT=bridge-hoodi-<service>.service`; paid-worker child logs and durable state remain under `run/hoodi-v210`. Protected evidence is under `/srv/vara-bridge/evidence`. The pre-start cold archive and logical database dump remain under `/srv/vara-bridge/backups`; retain them together with their recorded hashes. The local cold validators and journals remain fenced. Once remote state advances, rollback requires fencing remote writers, reconciling originals, making a fresh cold transfer and database dump back, and resuming exactly one owner. Starting the stale local snapshot is never rollback.

Nginx serves only immutable static releases under `/usr/share/nginx/html/bridge-hoodi/releases`, with an atomic live-route include, uncached HTML and immutable hashed assets. Copy complete releases, set readable ownership/modes, and use `restorecon -RF` to replace preserved container SELinux labels before switching; run `nginx -t` before reload. Proxy `/vara` only to Safe RPC 52549 and `/graphql` to 5450, with upstream Host headers, exact-origin checks, HTTP/1.1 upgrades and stripped Basic credentials. `/faucet/` proxies to loopback 5485 under the same authentication, strips credentials/cookies, limits bodies to 1 KiB and disables upstream retries. All other bridge listeners remain private.

Retrieve site access only over SSH with `cat /home/bridge/.config/bridge-hoodi/site-access`; do not put credentials in URLs, frontend environment variables or reports. The separate root `bridge-hoodi-cert.timer` renews through Lego state `/var/lib/lego-bridge-hoodi-temp` and `/usr/local/libexec/bridge-hoodi-cert-deploy`, preserving the existing managed certificate jobs. HTML requests the web manifest with `crossorigin="use-credentials"` so it remains protected rather than exempting it from authentication.

`VITE_ISOLATED_FAUCET=true` enables the retained lane's same-origin faucet in Testnet mode. It grants **100 TVARA** or **1,000 Test USDC** per normalized address per 24 hours. USDC is the existing six-decimal ERC20 `0xd4d65b1100b522cbde9664b9ba78c8add04e5b53`, paired with six-decimal hUSDC VFT `0x4e1512f4e996cf4784d052661f38c31b39606a1716f0f735e6aec64df5249baf`. These are test-only assets with no monetary value, not real Circle USDC. The faucet transfers canonical ERC20 inventory; it never independently mints VFT.

The [faucet service](../tools/test-faucet.cjs) uses two dedicated signers, with initial pools capped at 10,000 TVARA and 100,000 Test USDC plus 0.005 Hoodi ETH for gas. There are no automatic refills. Allocation ceilings are lifetime bounds; fees can exhaust funding earlier. Address limits are not person-level Sybil protection. One unresolved original holds further claims for that asset until canonical finality, including Hoodi's finality delay. The other asset remains independent.

The task-owned launcher `/srv/vara-bridge/config/run-faucet` mounts only read-only application code/configuration, dedicated keys under `config/faucet-keys`, and the faucet state directory. Configuration `config/faucet.json` and key files require mode 0600; directories require 0700. Its `stateDir` maps to `run/hoodi-v210/faucet/journal`, which must not exist before first initialization. Preserve this authenticated journal and both keys together. Never restore an older journal, delete an unresolved original, rotate signers in place, or run another host against these signers. A nonce/history mismatch or interrupted journal fails closed.

`GET /faucet/config` publishes the exact genesis, chain ID, asset amounts and cooldown. `POST /faucet/request` accepts only `{asset,address,genesis,chainId}` with the exact site Origin; `GET /faucet/status?asset=…&address=…` recovers an existing outcome. Originals are durably saved before one broadcast attempt. A pending repeat returns the same transaction; restart observes without rebroadcasting or replacing it. A completed daily claim returns 429. The UI requires the approved Gear genesis, additionally requires a connected Hoodi wallet for USDC, and does not treat a 202 response as finality. After a lost response, inspect status rather than blindly submitting again.

Selecting Testnet changes application RPCs, not the connected wallet's chain. If Rabby or another wallet still reports Ethereum mainnet (chain 1), the USDC faucet shows that chain ID and offers **Switch wallet to Hoodi**, reusing the bridge's existing wallet-network synchronization. This action does not submit a faucet claim or sign a transaction. Claims remain disabled until the connector reports Hoodi (560048).

Outside this opt-in lane, missing Turnstile configuration still disables the captcha-backed faucet instead of mounting an unconfigured widget. The independent Hoodi ETH faucet link remains available with an Ethereum account. Repeated browser authentication prompts are deferred, not fixed. Reown currently rejects the temporary origin with `3000 (Unauthorized: origin not allowed)`: the configured project's owner must authorize this exact HTTPS origin before WalletConnect can be called ready. The installed Polkadot JS injected-account path works without that external prerequisite. Mainnet's external explorer also has its own origin policy; this retained deployment is qualified in Testnet mode only.

#### Snowbridge lineage and deployment boundary

[Interop fixtures](../tools/beefy-relay/src/fixtures.rs) pin Snowbridge commit `1201293e482ef052b9c3989dcf680046704fef3d`. This lane differs through a direct Vara queue snapshot instead of BridgeHub/parachain proofs, destination binding, authenticated source freshness, tiny-set quorum and existing Vara queue and administrative semantics. Retain `MAX_VALIDATORS=256` and both signature ceilings at 86, capped by `floor(N/3)+1` with native small-set quorum rules. N=59 requires 20 EVM signatures and native quorum 40; N=256 requires 86 and 171 respectively. Both EVM paths require distinct positions and accounts; authenticated native rosters require distinct source keys and derived EVM accounts. N=2 requires both signatures. These bounds and synthetic fixtures do not qualify the actual production roster or transfer upstream audit conclusions.

The architectural reference for this finalization is Snowbridge `630c2081359e2417496e7f4c1a189a7eb40f7bd4`; the older interoperability-vector pin above remains distinct. Neither reference transfers an upstream audit to these adapters.

`Source::connect_pair` authenticates both RPCs at one common finalized upgraded pin: runtime/API identity, separately labeled source `:code` hashes, current/next authorities, activation `G`, actual first insertion `S`, MMR root/count and destination-bound domain. The first-leaf proof establishes insertion geometry; activation is not read from block zero and missing activation is HOLD. New anchors retain `domainBindingBlock` and `sourceIdentity`; old anchors remain restricted to the explicit legacy runtime. Public artifact, validator-roster, weight, custody and migration qualification remain separate gates. `migrateToBeefy` preserves existing UUPS authority and legacy replay/root state while separately binding the reviewed timelock's narrow expiry-recovery capability; relay signers receive neither. See the [recovery policy](../ethereum/README.md#expiry-recovery-and-initializer-scope) and [mainnet migration gates](../ethereum/README.md#mainnet-custody-preserving-migration-candidate).

## Process bootstrap

[relayer/src/main.rs](../relayer/src/main.rs) creates a multi-thread Tokio runtime with a bounded blocking pool, loads dotenv/logging, and dispatches the selected Clap command. It does not initialize a circuit pool or a proof-storage service. The retained commands are `eth-gear-core`, `gear-eth-tokens` and `eth-gear-tokens` (each with `all-token-transfers` and `paid-token-transfers`), `gear-eth-manual`, `eth-gear-manual`, `kill-switch`, `queue-cleaner` and `fetch-merkle-roots`.

The selected worker owns its configured clients, listener/channels, metrics and storage. BEEFY root production remains in the existing `beefy-relay` actor, not a replacement scheduler inside `relayer`. Shared connection and signer arguments retain dotenv/environment loading; consult each compiled command's help and the [running guide](running-the-bridge.md).

## Gear finality and block delivery

[BlockListener](../relayer/src/message_relayer/common/gear/block_listener.rs) consumes GRANDPA justifications rather than arbitrary best-head blocks. It fetches finalized block evidence, extracts queue changes and authority annotations, persists through the consumer's existing block-storage interface, and broadcasts to retained discovery workers. Storage determines whether full finality material is needed; this is not an outbound recursive proving stage.

Startup replays unprocessed ranges. Live justification gaps replay missing finalized blocks before newer observations; recoverable provider errors reconnect from the retained cursor. GRANDPA subscriptions and authority annotations remain necessary for discovery and queue cleaning even though outbound root authentication uses BEEFY/MMR. Persisted evidence is recovery state, not a disposable cache.

## Gear-to-Ethereum root flow

1. The [follower](../tools/beefy-relay/src/follower.rs) authenticates the source/witness identity and ordered native BEEFY commitments, including mandatory authority handovers and freshness updates.
2. The [source reader](../tools/beefy-relay/src/source.rs) reconstructs the exact historical queue snapshot and its MMR inclusion under an accepted covering commitment, rather than substituting the current queue root.
3. The [root publisher](../tools/beefy-relay/src/root_publication.rs) retains the original publication identity and submits the canonical v2 envelope to `MessageQueue`; the typed [verifier](../ethereum/src/VaraQueueRootVerifier.sol) checks it against the latest live `BeefyClient` anchor.
4. Finalized canonical acceptance is reconciled against the original transaction and on-chain root before progress is recorded. Unsigned external progress and signed-original outcomes remain distinct.
5. The retained token/manual delivery pipeline supplies an individual-message Merkle proof against the registered historical root after its own maturity delay. A shared commitment does not merge roots, eliminate per-block publication costs or prove settlement.

The demand scheduler, idle heartbeat, original-transaction fencing, bounded-distance catch-up and history-retention limits are described above. There is no priority-proof scheduler, recursive proof storage or HTTP root-proof producer.


## Ethereum-to-Gear core

[relayer/src/ethereum_checkpoints/](../relayer/src/ethereum_checkpoints/) contains the core Ethereum-to-Gear relayer. The eth-gear-core command creates a beacon client, a Gear client, and a checkpoint-light-client relayer using:

- the checkpoint-light-client program id;
- an Ethereum beacon RPC endpoint;
- a Gear signer URI and endpoint;
- a slot-batch multiplier.

The relayer follows finalized beacon-chain data and submits sync-committee/finality updates to the Gear light-client program. Token/event consumers can then ask the Gear-side eth-events-* programs and historical-proxy to verify a transaction or event.

Bootstrap requires an independently trusted recent checkpoint root, not the deployment provider's choice of finality. `checkpoints-tool` requires that root and the trusted genesis validators root; the actor checks bootstrap proofs against the supplied root and rejects period-zero bootstrap. The selected `Network` fixes genesis validators identity, genesis time and the Deneb/Electra/Fulu schedule for the lifetime of the actor.

Regular updates and replay enforce `current_slot >= signature_slot > attested_slot >= finalized_slot`, sequential authenticated committee transitions and the fork at the preceding signature slot. Replay headers must already be strictly ordered and chain to the authenticated interval; neither the relayer nor the actor sorts malformed input into validity. Revision compare-and-set, bounded checkpoint history and native BLS verification remain enforced.

Checkpoint deployment, updates and native integration tests submit with 95% of the block gas limit. Deployment and native-test gas estimates must fit that ceiling; the exact RPC minimum is not an execution budget. It can exhaust gas during queued BLS execution even when the update is valid.

This process is distinct from the Ethereum token relayer: one advances the light-client state, while the other submits user/event receipts through the programs that consume that state.

## Observer challenges and governance response

The independent [kill switch](../relayer/src/kill_switch.rs) holds only its observer-role signer. It connects canonical destination events, including hot blocks, to a pinned finalized ancestor and verifies stored roots at the pinned head before comparing the exact finalized historical source root. The cursor advances only through finalized history, so hot blocks and their replacements are revisited. Removed/inconsistent logs, invalid source bounds/finality and unavailable historical state hold the cursor without authorizing a challenge. Once a mismatch is authenticated, later unavailable source state cannot suppress it. The operator-configured source RPC remains a trust boundary, not a second source-consensus verifier.

A qualified mismatch sends `challengeRoot()` directly. Once a challenge hash exists, receipt retries retain it; RPC uncertainty never authorizes another transaction. A finalized revert requires a fresh scan before another submission. A finalized successful challenge exits for operator intervention, with no automatic root repair or unpause and no admin key or repair URL. Reconcile original evidence before restarting.

The challenge pause expires after `CHALLENGE_ROOT_DELAY` (two days); it is not an indefinite incident lock. Persistent incidents require the existing separately authorized governance pause. Any emergency-admin action, authenticated root publication, challenge removal, verifier expiry recovery or governance unpause is a separate authorized response, not work performed by the observer. Recovery retains its reviewed positive-delay timelock and forward-history restrictions, and cannot reset custody, registered roots or replay state.


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

The implementations under [relayer/src/message_relayer/eth_to_gear/](../relayer/src/message_relayer/eth_to_gear/) retain the original signed Gear dispatch, receipt bytes, request/reply linkage and per-log effects. `Processed` alone never completes an entry. Completion requires the exact canonical finalized original proxy/consumer reply and every authenticated settlement row. An original `NativeSettlementPending` may resolve through its original payout becoming `Delivered` with zero returned value or `Refunded` with its exact full value returned and atomically credited as wrapped VARA to the original Gear recipient. A `Queued` payout with its exact full backing returned authorizes a separate zero-value `ReconcileReceipt`: the manager continues the same `RedeemEscrow` operation to credit the wrapped refund, without sending another native payout or resubmitting the original proof. Both the Rust relayer and SDK distinguish this continuation-ready state from completion, which still requires the terminal payout, matching `Settled`/`Refunded` rows and `Processed` receipt state. Reconciliation never rewrites the original reply. Missing history, unknown outcomes and arbitrary inner errors HOLD.

Finalized original/continuation inclusion releases account-nonce ownership, not
economic evidence. Pending payouts remain fairly reconciled while unrelated fresh
receipts progress; unknown originals and fresh continuation allocation remain
serialized. Ethereum ancestry uses bounded per-request deadlines and a cooperative
checkpointed traversal, not a 30-second cancellation deadline around the entire
multi-request proof.

Receipt verification consumes the complete SCALE frame and receipt RLP, requires a successful EIP-658 status, and checks exact SSZ branch depth. Historical headers must ascend within `(receipt.slot, checkpoint.slot]`, connect through parent roots and terminate at the exact authenticated checkpoint; empty ancestry is valid only at that checkpoint. Receipt endpoints enforce the immutable network fork schedule. The historical proxy authenticates the requested slot and verifier reply but preserves the consumer's raw reply bytes: outer proxy success does not establish a successful inner application result or economic settlement.

Checkpoint-waiting proof work is keyed by its original transaction UUID. Repeated notifications for the same hash and slot are idempotent; conflicting identities HOLD without replacing the original. Eligible work moves to the processing queue when its checkpoint arrives, and composition failures retain that work across reconnects.

Ordinary Gear-origin returns use escrow `TransferFrom`. Native policy is explicit and snapshotted per deposit: the wrapper burns manager escrow and retains the original payout child. Zero minimum balance preserves fractional pooled escrow. Only the authenticated original recipient's full error return permits a one-time backed wrapped refund; partial returns, queued payouts without returned backing, ambiguous payouts and retained legacy Returned outcomes remain held. A full return alone is not a completed refund. Generation/lease and per-log ownership prevent redispatch. Source reconciliation still requires original request/child/hash and authenticated builtin outcome, never timeout-based “not queued” inference.

SDK callers supply an immutable, independently approved `InboundProofProfile` binding both network identities, the source runtime and block, actor CodeIds, exact IDL digests, endpoint framing, checkpoint network, forks and consumer route. Frontend completion uses `onFinalized`, not an outer proxy success flag. Outbound SDK callers supply the exact expected effect from the packed original source message. Missing profiles or deployment configuration render HOLD before signing.

The Ethereum indexer uses cold-only ingestion with a 75-block scheduling buffer; canonical finalized headers and anchor are still authenticated before effects/cursor commit. Ethereum completion joins exact queue and token effects in one original receipt. Gear `ReceiptDepositSettled` and distinct `ReceiptDepositRefunded` rows correlate slot, transaction index and receipt-local log index, preserving first-log identity. Refund evidence additionally binds original operation, wrapper, recipient, manager/payout children and full returned backing; it projects `Refunded`, not native `Completed`. Bounded batched joins cycle through at most 1000 pending settlements so late counterparts remain eligible. Apply `1792022400000-native_refund.js` before exposing refunds on an existing database, restart its API and regenerate frontend GraphQL types; no shared deployment migration is executed by these code changes. Legacy completion rows require original-evidence requalification.

The refund down-migration refuses while either `transfer.status` or retained
`receipt_settlement.deposit.outcome` records `Refunded`, including unmatched
evidence whose transfer counterpart has not arrived yet.

Both indexers hold the entire database callback while an otherwise valid finalized head lags the batch. They poll every 30 seconds without a five-attempt ceiling, reauthenticate the previous height/hash before replacing it, and reject regression, malformed responses, missing history or canonical conflicts. Gear also disables hot-block ingestion. Bounded RPC timeouts remain; process shutdown cancels waiting. Neither projection effects nor the processor cursor commit before the full batch and final anchor validate.

Retained Ethereum indexer checkpoint catch-up checks every intervening parent on both providers. The finite historical walk uses a fresh 10-second timeout for each RPC rather than the 300-second fresh-tip qualification deadline, so checkpoint age alone cannot prevent recovery. Broken links, missing history and stalled requests still HOLD; the candidate anchor is rechecked before it replaces the retained checkpoint.

Outbound queue receipts that arrive before their Vara counterpart are retained in `CompletedTransfer.outboundReceipt`, not discarded or marked complete. Migration `1791936000000-outbound_completion.js` adds this authenticated-receipt evidence and the bounded pending-match index. Each later batch rechecks the original canonical finalized inclusion, exact message identity and same-receipt token effect before completing the late counterpart. Replayed receipts compare structurally across PostgreSQL JSONB key ordering. Legacy rows without this evidence cannot authorize completion.

The GraphQL `Transfer.amount` resolver emits an exact base-10 integer string, including for preserved database rows stored as hexadecimal strings. It does not rewrite historical rows or convert token amounts through JavaScript floating-point numbers.

For an existing indexer database, applying the schema migration is not a data cutover.
Inbound reconciliation can backfill NULL transaction/receipt-local log coordinates
from the independently authenticated original receipt, requiring a unique matching
manager deposit and preserving every known coordinate. `--apply` retains this source
evidence even while settlement is pending; a dry run writes nothing. Completion still
requires the original receipt relay and authenticated settlement, which also establish
the receipt slot. Missing indexed history requires an approved reindex/backfill;
never derive coordinates from concatenated nonces or manufacture Settled rows.

Retained Gear manager routing requires migration `1792454400000-retained_manager_handoff.js`
before starting the updated indexer (`yarn workspace indexer migration:run` against
the approved database). Set `GEAR_RETAINED_CUTOVER_FROM_BLOCK` to an approved Gear
block where the persisted manager supports the current `CutoverStatus` query,
after any legacy exit/inheritor rotation and before the first retained handoff.
Leave it unset only for legacy exit-based routing. The RPC must support historical
`gear_calculateReplyForHandle` at finalized block hashes, and `GEAR_API_PATH` must
contain the current manager IDL; query failures at or after activation HOLD the
batch rather than silently falling back to legacy behavior.

The existing `gear_programs` manager role remains the trusted lineage root;
changing `GEAR_VFT_MANAGER` does not replace it. The indexer authenticates reciprocal
source/successor state, receipt boundaries, custody completion and matching
fingerprints before admitting the successor, without requiring the retired source
to exit. Handoff evidence, projected effects and the processor cursor share one
database transaction. Predecessor history remains routable within its receipt
domain, and replay cannot use a successor authenticated only at a later block.
Once handoff evidence exists, changing or removing the activation boundary fails
closed. If an older indexer already skipped successor events, use an approved
reindex/backfill from before the handoff; restarting at its advanced cursor or
changing the configured manager ID cannot recover missing history.

The beacon/light-client path and the event/message path are complementary:

1. eth-gear-core advances checkpoint-light-client state.
2. The token relayer asks historical-proxy/eth-events-* to prove an event against that state.
3. The VFT manager mints, unlocks, or transfers the corresponding Gear-side token.

## HTTP routing and channels

The shared HTTP server in [relayer/src/server.rs](../relayer/src/server.rs) uses the X-Token header for authentication. It exposes only the routes enabled by the caller:

- /relay_messages sends Gear message descriptors to a Gear-to-Ethereum token relayer;
- /relay_transactions sends Ethereum transaction hashes to an Ethereum-to-Gear token relayer.

The server deduplicates repeated items within one request and retains bounded request bodies. It returns 401 for a missing or incorrect token, 200 when all items were accepted/handled, 202 for partial acceptance, and 500 when no item could be queued or a response channel failed.

Each selected paid worker creates its own listener and optional route channel; unavailable routes are not enabled. Listener endpoint identity and port isolate workers and networks; JSON requests do not choose a network. Only `/relay_messages` and `/relay_transactions` remain, not a root-proof HTTP API.

## Persistence and failure boundaries

The main persistent boundaries are:

- BEEFY follower and root-publication journals, original signed bytes and nonce ownership, source identity pins and authenticated finality archives.
- Retained finalized source discovery state and queued/paid-event cursors.
- Ethereum block/transaction and original message-delivery evidence for token/manual relayers.
- Inbound original Gear dispatch, proof-composition work and settlement reconciliation state.

Atomic persistence and finalized-state reconciliation provide idempotency, not permission to delete history. Root-publication holds may release independent authenticated commitment following without discarding the held original. Preserve the existing journal owners, finality fences, archive evidence and signer locks on restart.

RPC errors are classified at the provider and listener boundaries. Recoverable transport/subscription errors trigger retries and reconnects; permanent errors or exhausted retry policies are returned to the owning service. When a service channel closes, the parent relayer treats that as a component failure rather than silently continuing with incomplete proof or submission state.

## Module map

| Concern | Main implementation |
| --- | --- |
| CLI and process supervision | [relayer/src/main.rs](../relayer/src/main.rs), [relayer/src/cli/](../relayer/src/cli/) |
| CLI and environment configuration | [relayer/src/cli/](../relayer/src/cli/) |
| Gear finalized-block delivery | [relayer/src/message_relayer/common/gear/block_listener.rs](../relayer/src/message_relayer/common/gear/block_listener.rs) |
| BEEFY commitment following | [tools/beefy-relay/src/follower.rs](../tools/beefy-relay/src/follower.rs) |
| Historical source/MMR proofs | [tools/beefy-relay/src/source.rs](../tools/beefy-relay/src/source.rs), [tools/beefy-relay/src/protocol.rs](../tools/beefy-relay/src/protocol.rs) |
| Authenticated root publication | [tools/beefy-relay/src/root_publication.rs](../tools/beefy-relay/src/root_publication.rs) |
| Observer challenges | [relayer/src/kill_switch.rs](../relayer/src/kill_switch.rs) |
| Ethereum beacon/light-client relay | [relayer/src/ethereum_checkpoints/](../relayer/src/ethereum_checkpoints/) |
| Gear-to-Ethereum token relay | [relayer/src/message_relayer/gear_to_eth/](../relayer/src/message_relayer/gear_to_eth/) |
| Ethereum-to-Gear token relay | [relayer/src/message_relayer/eth_to_gear/](../relayer/src/message_relayer/eth_to_gear/) |
| Solidity authentication and queue | [ethereum/src/beefy/BeefyClient.sol](../ethereum/src/beefy/BeefyClient.sol), [ethereum/src/VaraQueueRootVerifier.sol](../ethereum/src/VaraQueueRootVerifier.sol), [ethereum/src/MessageQueue.sol](../ethereum/src/MessageQueue.sol) |
