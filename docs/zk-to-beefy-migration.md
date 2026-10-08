# ZK to BEEFY: custody-preserving migration runbook

**Decision: keep the deployed MessageQueue proxy, ERC20Manager, existing Gear VFT managers and token programs. Change outbound root authentication in place, through the existing governance route, only after the gates below pass.** Do not transfer custody to a new manager or reset either replay ledger.

**Current verdict: BLOCKED. This document and its offline proposal tool do not authorize a deployment, signature, broadcast, source-runtime activation or custody move.** The local BEEFY/Hoodi token lane is a separate test deployment; its genesis, keys, domain, wallets and journals cannot be substituted for a public ZK deployment. Completing its four-token campaign does not clear the gates here.

## 1. Inventory and evidence boundary

The companion [deployment inventory](zk-to-beefy-inventory.json) distinguishes `mainnet` from `publicHoodi`. Select one; never combine their addresses, balances, checkpoints or nonces. Each observation records `value`, `status`, `provenance`, `observedAt` and `snapshot`:

- `measured`: the named provider answered a query at a recorded canonical finalized hash. It is not independent authentication or proof that checked-in source matches deployed bytecode.
- `advertised`: documentation, configuration or a published address. It is not a chain observation.
- `inferred`: interpretation from source or another observation.
- `unknown`: missing evidence; `unavailable`: a failed query. Neither means zero, empty, false, inactive-without-liabilities or safe.

A migration package must hash-pin that inventory and the evidence artifacts described in section 8. Refresh expired observations before execution; do not edit a historical snapshot to make it agree with current state. Pin one EL block hash for all EVM comparisons and one finalized Gear hash for all Gear comparisons. Use archive providers, a separate witness, local proof/header verification and the correct network's Beacon finality. A provider's `finalized` label alone is not the finality gate.

### Present reconciliation HOLD

The inventory investigation reported these **unreconciled observations**, not an insolvency conclusion:

| Mainnet asset | Custody raw units | Corresponding wrapped supply raw units | Custody minus supply |
|---|---:|---:|---:|
| USDT, Ethereum-origin | 14233502454 | 14233515874 | -13420 |
| WETH, Ethereum-origin | 44347022692197676 | 42803087151958221 | +1543935540239455 |
| WVARA, Gear-origin | 374987275288002004 | 346624259316796904 | +28363015971205100 |
| USDC, Ethereum-origin | 1000000 | 1000000 | 0 |
| BTC, Ethereum-origin | 0 | 0 | 0 |

The EL pin was mainnet block **26097933**, hash `0xad48f1b969a5c92e3b5bc70ac43e9f2efb0aa180ff07e7c6a955148b7d60074b`. Gear pins and complete timestamps/hashes are in the inventory. The finalized blocks’ own timestamps were **22 minutes 19 seconds apart** (EL 14:01:59 versus Gear 14:24:18); the query observation times were about **3 minutes 37 seconds apart**, which is a different skew. Either interval can contain real in-flight transfers, but **neither is an explanation without matching canonical transfers and per-effect accounting**. Origin-through-cutover ledgers are missing. All three residuals therefore hold migration. Do not net USDT against WETH or VARA, assume a pending amount, replenish custody, mint supply to balance the table or infer insolvency from these unmatched cuts.

Current mainnet EVM observations differ from older address lists: the queue’s measured verifier is `0xb7142e82ceead0df5d0b3507240a503e99e1881e`, not the advertised older verifier; its `recoveryController()` call reverts, which is **not** a zero-address observation. The ERC20Manager still authorizes three historical VFT-manager IDs. The retired manager chain `e01… -> c97… -> 440…` and native token chain `dbf80… -> 29c42…` require their original migration exports/imports, exit destinations, custody and replay evidence. An `Exited` or `InactiveProgram` result does not extinguish a claim. The emergency admin is an observed EIP-7702 delegation designator to `0x63c0c19a282a1b52b07dd5a65b58948a07dae32b`; Safe getters revert, so independent Safe authority is not established.

The live source scan observed queue ID 700, next global nonce 865 and an initialized/unpaused source bridge. Of 865 `isProcessed` queries for nonce 0–864 at the EL pin, 128 were false; they are not all proven asset liabilities. An indexer supplied 137 non-completed discovery records, not an origin-complete ledger. A zero current queue/root getter does not erase older nonempty roots or redemption rights. The deployed current manager returned `Unknown call`/userspace panic for the candidate source’s new `ReceiptStatus` method; that is an ABI/query failure, **not receipt state `Unknown`**. Its 499 enumerated transaction keys do not establish reserved/processed/per-log history. Both current and still-active historical proxies/endpoints, including the measured `bdc6…` checkpoint rather than the older advertised `f0a…` checkpoint, remain in scope.

Inventory **every** EVM `tokens()` entry, Gear mapping, active wrapped program and historical custody/replay owner, including native VARA and fees. This is not limited to the isolated lane's USDC/USDT/WETH/WBTC test assets. Zero supply still requires verified identity, mapping and ledger coverage.

## 2. Existing capabilities and their limits

These are source-backed capabilities, not claims that the live proxies use this exact release. Bytecode, proxy slots, layout and program-code correspondence remain a gate.

| Owner | Existing API or behavior | Migration consequence |
|---|---|---|
| [MessageQueue](../ethereum/src/MessageQueue.sol) | UUPS `_authorizeUpgrade` requires `DEFAULT_ADMIN_ROLE`; that role is initialized to GovernanceAdmin. No generic verifier setter. | There is an upgrade route, but no callable ZK-to-BEEFY initializer in the current source. An audited migration release is a prerequisite, not an invented command. |
| MessageQueue | Retains `_blockNumbers`, `_merkleRootTimestamps` and global `_processedMessages[nonce]`. `processMessage` verifies an already-stored root without consulting the current verifier. | Previously registered roots can remain redeemable after a verifier cutover, with original maturity and nonce replay protection, if storage and handler bindings are preserved. |
| MessageQueue | `pause()` stops ordinary message processing; governance-source messages bypass pause. Root submission is not paused by that flag. Active root challenge blocks even governance processing. | Pause is not a root-publication fence. Do not use `challengeRoot` as a routine cutover lock; it can disable the upgrade path. |
| MessageQueue | User/pauser maturity is 300 seconds; admin maturity is 3600 seconds. Root progress is limited to 57600 source blocks per advance. | Preserve maturity timestamps. Pre-register governance roots and satisfy existing limits; no accelerated admin processing or fabricated empty-root bootstrap. |
| [ERC20Manager](../ethereum/src/ERC20Manager.sol) | `pause()` stops `requestBridging` ingress; `handleMessage` still performs valid releases. Its queue address is fixed. | Pausing deposits does not freeze redemption accounting. Preserve the manager and refresh until the queue cut is also frozen. |
| ERC20Manager | No custody sweep, withdrawal, queue setter, old-manager approval or VFT-manager removal API. Governance can add a VFT manager or register/create a token. | A new manager cannot pull escrow with a nominal `transferFrom`. Adding a manager does not import replay state and may create a second receipt consumer. |
| [GovernanceAdmin](../ethereum/src/GovernanceAdmin.sol) | Only its bound queue may call `handleMessage`, with the exact configured Gear governance source. Packed commands pause/unpause the three supported proxies or invoke `upgradeToAndCall`. | EVM admin is a contract, not a guessed wallet. Proposal approval must reach the real Gear source and then the authenticated queue. |
| [GovernancePauser](../ethereum/src/GovernancePauser.sol) | Only the bound queue plus configured Gear source; pause/unpause, not upgrades. | A pause authority is not an upgrade or custody authority. |
| [RecoveryController](../ethereum/src/RecoveryController.sol) | 24-hour recovery; validates an expired old BEEFY client and a forward candidate with identical lane identity. Installation requires an existing recoverable BEEFY verifier. | Not a ZK migration adapter. Do not claim the legacy controller exists, treat a reverting getter as absence, or bypass its timelock. |
| [Gear VFT manager](../gear-programs/vft-manager/app/src/services/mod.rs) | Admin can pause, change mapping/proxy/manager/config, and change admin/pauser. `upgrade(newManager)` transfers manager-held balances and exits after latching the target. | This is program replacement, not storage-preserving code upgrade or automatic user/replay-state migration. It is not selected here. |
| [Receipt processing](../gear-programs/vft-manager/app/src/services/submit_receipt/mod.rs) | Separate `Unknown`, `Reserved`, `Processed` state, per-log progress and oldest-key fence; current local source rejects an ambiguous reserved receipt. | Preserve the original consumer and all replay/progress state. `Unknown` is not permission to mint; `Reserved` is not `Processed`. Deployed older behavior must be established separately. |
| Gear VFT manager | `transactions` merges processed and reserved keys. `insert_transactions` marks keys processed; bounded retention may trim oldest keys. | A transactions dump alone is not a safe migration export. Do not convert reservations to successes or import only a convenient suffix. |
| [HistoricalProxy](../gear-programs/historical-proxy/app/src/service.rs) | Admin appends increasing slot endpoints; `Redirect` verifies proofs and forwards to the caller-selected client and route. It has no global receipt-consumer replay ledger. | Keeping endpoints does not prevent two managers consuming one receipt. Keep exactly one authorized economic consumer per asset/receipt namespace and retain historical endpoints. |
| [Checkpoint program](../gear-programs/checkpoint-light-client/app/src/lib.rs) | Cryptographic checkpoint/sync/replay-back updates; no admin checkpoint reset or network setter. Event-verifier checkpoint binding is initialized in its program. | Preserve old checkpoint and event-verifier programs and archive history. Fork/network changes need their own verified release; no fabricated checkpoint or network rebinding. |
| VFT services | Existing token admin sets minter, burner, pauser and admin; manager exit does not migrate users' balances. | Keep program IDs, balances, total supplies and role owners. A role change is not proof that old receipt state has been fenced. |

The queue's recovery fields were appended in the candidate source. That comment does not establish live layout compatibility. Capture the deployed implementation, compiler/OZ storage namespaces and historical layouts before proposing a release.

### Why not create a new queue or manager?

The [message hash](../ethereum/src/libraries/Hasher.sol) is `keccak256(nonce32BE || source32 || destination20 || payload)`; it does not include a source genesis or destination domain. A new queue starts with an empty processed-nonce mapping. A new Gear manager starts with a different receipt replay namespace. Reusing old proofs in either can therefore produce duplicate release or mint even when the proof itself is valid.

The selected cutover keeps the original replay owners and custody. If verified live layout or authority makes that impossible, **stop this plan**. A separately audited export/import, enforceable old-consumer retirement and authorized custody-transfer release would be required. Current `deploy-upgraded`, `migrate-transactions`, `--ignore-non-empty-message-tracker`, token exit or mapping replacement are not an end-to-end substitute.

## 3. Required ledgers and per-asset equations

Reconstruct from deployment origin, not a recent worker cursor. Establish the original EL discovery/deployment block and Gear deployment/runtime activation history. Recover archival events, receipts, queue snapshots, proofs, program migrations and worker journals through the frozen cuts. Reconcile independent providers with pinned hashes; reconcile worker records to chain effects, not the reverse.

### Ethereum to Gear ledger

For every canonical `BridgingRequested` log record:

- EL chain/genesis, manager, transaction hash, block number/hash, transaction index and log index;
- sender, recipient, token, raw amount and matching receipt RLP/proof/checkpoint/historical endpoint;
- original `(Beacon slot, transaction_index)` receipt key and original Gear consumer/program ID;
- every matching receipt log's mint/unlock effect, final Gear block/hash, message/reply ID and supply/balance delta;
- `Unknown`/`Reserved`/`Processed`, per-log progress, processed retention floor, and definitive failure versus ambiguous effect;
- original worker intent/handoff/cursor and immutable message identities. Store references/digests for signed bodies, never copies in this public package.

One receipt can contain several deposit logs. Count effects per log, but enforce replay per original receipt key/consumer. A transaction-level success flag or a manually imported `Processed` key cannot stand in for the actual mint/unlock effects. A partial mint must never be treated as wholly unminted and resubmitted.

### Gear to Ethereum ledger

For every request, payment and queued asset message record:

- original source genesis, manager and token mapping; request message/replies; asset amount and actual lock/burn/refund effects;
- normal/priority fee, payment nonce/event and missing request/payment counterpart;
- queue ID, globally allocated message nonce, message source/destination/payload/hash and finalized source block;
- queue snapshot, total leaves/index/inclusion proof, ZK root proof, registered root block/root and original maturity timestamp;
- worker intent, original EVM sender/transaction nonce/hash, inclusion, canonical receipt/finality and `isProcessed(messageNonce)`;
- classified terminal release, confirmed no-queue refund, or still-redeemable registered claim. All ambiguous, reserved, unpaid/pair-missing, unregistered-root or missing-proof cases remain HOLD.

Include governance messages in a separate control ledger. Asset and governance watermarks differ because the source bridge permits governance queueing while paused. A forgotten old upgrade/unpause/mapping message can invalidate the cutover after apparent success.

### Equations at consistent frozen cuts

Use integers in raw units. This bridge copies amounts; it does not rescale decimals. Require matching token/VFT decimals and prove each origin/type mapping. No floating point or cross-asset netting.

For **Ethereum-origin** asset `i`:

```text
EVM manager escrow_i
  = Gear wrapped total supply_i
  + finalized EVM locks_i whose corresponding mint effect has not occurred
  + completed Gear burns_i whose EVM release has not occurred
  + separately proven non-liability surplus_i
```

For **Gear-origin** asset `i`:

```text
Gear custody_i across every attributable current/historical holder
  = EVM wrapped total supply_i
  + completed Gear locks_i whose EVM mint has not occurred
  + completed EVM burns_i whose Gear unlock has not occurred
  + separately proven non-liability surplus_i
```

A pending amount is included only for a matching canonical effect and original identity. Interrupted operations awaiting refund or with uncertain effect must be reconciled before cutover, not assigned a balancing number. Native VARA additionally needs its native-exchange backing, minimum balance, program exits/refunds and account balances reconciled; spendable gas and bridge fees are not backing. Include ERC20 fee-payment ownership/native balances and Gear fee-program balances separately.

Prove the cumulative deltas too:

```text
Ethereum-origin: delta escrow = locks - releases
                  delta wrapped supply = mints - burns + confirmed refund-mints
Gear-origin:     delta custody = locks - unlocks - confirmed refunds
                  delta EVM wrapped supply = mints - burns
```

Explain every authorized non-bridge mint/burn, transfer, token migration, donation or supply adjustment from canonical history. An unknown historical baseline, unexplained positive residual or negative residual holds that asset and this whole cutover. A historical deficit is not repaired by changing the manifest. The selected execution requires all inbound effects settled before the switch; outstanding outbound claims may remain only if their **original roots, timestamps, inclusion proofs, custody and replay owners** remain redeemable.

## 4. Authority and unsigned proposal route

Before staging anything, record the actual chain authority graph:

1. Queue/manager UUPS implementations, code hashes, `DEFAULT_ADMIN_ROLE` and pauser-role members; GovernanceAdmin/Pauser code, queue bindings and Gear `governance()` values.
2. Source bridge admin/pauser pallet-account identities and the live runtime metadata. Mainnet's measured governance identity is a pallet account (`modl…gethb0bridge_admin…`), not evidence of a Safe or a privately signable EOA.
3. The exact on-chain proxy/multisig/root/referendum dispatch route able to produce the required **effective Gear origin**. Record signatories, threshold, proxy type/delay, call filters, deposits and execution delay. A generic `Governance` or `NonTransfer` proxy cannot be assumed to allow `GearEthBridge` calls; inspect the actual filters and nested-call authorization.
4. Every Gear manager admin/pauser, historical-proxy admin, VFT admin/minter/burner/pauser, fee admin and ERC20 wrapped-token owner. Retired actors require their recorded exit/migration destination and historical authority.
5. Emergency observers/admin and any recovery wallet/controller separately. A 23-byte account delegation, failed Safe getter or a local 3-of-5 Safe is not proof of independent custody governance. Resolve delegated code/authority where applicable.

The exact existing governance payloads are:

| Action | Effective Gear source | EVM destination | Packed payload |
|---|---|---|---|
| Pause ERC20 ingress | configured GovernancePauser source (or reviewed admin route) | GovernancePauser contract | `0x01 || ERC20Manager20` |
| Pause user redemption | same | GovernancePauser contract | `0x01 || MessageQueue20` |
| Atomic queue upgrade | configured GovernanceAdmin source | GovernanceAdmin contract | `0x03 || MessageQueue20 || newImplementation20 || initializerCalldata` |
| Resume user redemption — fresh authorization only after G4/G5 | configured pauser source | GovernancePauser contract | `0x02 || MessageQueue20` |
| Resume ERC20 ingress — fresh authorization only after staged G6 review | configured pauser source | GovernancePauser contract | `0x02 || ERC20Manager20` |

The source call is the metadata-bound `GearEthBridge.send_eth_message(destination, payload)`. Root authorization or a proxy/multisig may wrap it; do not guess its SCALE bytes or sign from a pallet account. Source/witness must independently observe the original `MessageQueued` identity. A worker or any other caller can use the valid public root/inclusion proof to submit normal `processMessage`, which reaches governance. Admin upgrade messages require the existing **one-hour** maturity; ordinary pauser messages require **300 seconds**. Governance bypasses queue pause, and an ordinary unpause payload has no postcheck predicate: once preauthorized and mature, it could execute before an upgrade or review. Therefore fresh resume authorization is forbidden until G4/G5 pass; an off-chain sequencing promise cannot guard it.

The [governance tool](../tools/governance/src/cli.rs) can display packed messages (`GovernanceAdmin PauseProxy ERC20Manager`, `GovernanceAdmin UpgradeProxy MessageQueue …`). It reads moving deployment state and a `deployment.toml`, not this package's finalized pins. Its output is proposal preparation, not execution or independent approval. The offline tool below avoids that moving-state dependency.

Gear manager pause is the existing `VftManager/Pause` Sails route from its real admin/pauser; source runtime pause is `GearEthBridge.pause` under its actual root/admin/pauser origin. Encode wrappers from pinned metadata/IDL and submit through the approved governance process only. Do not replace either with a direct unauthenticated manager call.

## 5. Missing migration release and public BEEFY prerequisites

### Audited in-place queue release

**No existing initializer signature is supplied or assumed.** `initializeWithRecovery` initializes a fresh queue; it is not a migration of an already initialized ZK proxy. `installRecoveryController` rejects a non-recoverable ZK verifier. `activateRecoveryVerifier` belongs to BEEFY recovery, not this cutover.

A separately reviewed release must provide a real ABI, reproducible implementation bytecode, code hash, compatible compiler/OZ layout and a one-time migration initializer. Its verified behavior must:

- execute as one `upgradeToAndCall` on the original proxy, under the existing GovernanceAdmin;
- compare the current verifier with the exact expected legacy verifier, rejecting stale or repeat execution; set exactly one new verifier, never ZK-or-BEEFY fallback;
- bind the new verifier/client to the original queue, destination chain, actual source domain/genesis evidence and public activation/MMR history;
- enforce a reviewed legacy source-root ceiling and strictly greater BEEFY root floor for future submissions, without deleting or disabling historical stored roots;
- preserve every stored root, original timestamp, genesis/max-block progress, processed nonce, role/binding, observer/challenge/emergency state and pause state; no initializer may reset mappings or replay floors;
- reject an active challenge/emergency or incompatible bootstrap rather than bypassing it; validate the client is live with an authenticated nonzero accepted commitment;
- keep ordinary ingress/redemption held until finalized postchecks; leave governance maturity and existing root-distance limits unchanged;
- install a reviewed independent recovery controller only if that release explicitly supports it and the actual public recovery authority passes its own gate. No local Safe inheritance by assumption.

Register the release's **actual** initializer signature, argument bindings and exact calldata in the migration package. The offline tool requires old-verifier, new-verifier and legacy-boundary arguments and checks the signature exists in the supplied ABI. If the reviewed ABI expresses those preconditions differently, adapt and review the preparer against that real release; do not fabricate an initializer to satisfy its schema. No release implementation is authorized by this runbook.

### Public source activation

The inspected source workspace is the local candidate at `../source/vara` relative to the bridge checkout; it is not proof of the public runtime's code or activation:

- Current relay discovery queries `BeefyApi_beefy_genesis` at block zero. Its bootstrap validator expects activation at block 1 and the local 3000-ms/64-slot BABE profile. A public post-genesis activation needs an authenticated activation-aware discovery implementation and historical descriptor, not a relaxed identity check or local raw spec.
- Source `BridgeDomain` is provisioned through genesis configuration in the candidate; no public activation/domain migration was found. Preserve the public source genesis, existing bridge queue/nonce and BABE/GRANDPA finality. Provision the destination-bound domain without applying the queue-reset migration, which kills `MessageNonce` and queue state.
- The session-key migration appends deterministic placeholder BEEFY bytes; it does not establish real validator-held signing keys. All public validators need valid keys, ownership, session installation and current/next-set agreement at the chosen activation boundary. Preserve their existing BABE/GRANDPA/im-online/discovery keys.
- Candidate runtime `MaxAuthorities` is 100000, while the Solidity BEEFY client supports `MAX_VALIDATORS=256`. `WeightInfo=()`/placeholder MMR weight is not public-load qualification. Public validator-set size, all signatures, handovers, secp256k1 keys, runtime weights and economics must be qualified before approval; no silent subset or security-constant change.
- The current Fiat-Shamir parameter is 86; larger-set selection remains `floor(N/3)+1` capped as implemented; two validators require both signatures. These parameters and two co-hosted test validators do not establish public-production security.
- Verify archive source state and offchain MMR nodes, the actual MMR start `S`, source block `B`, insertion `B+1`, commitment `C`, leaf index `B+1-S`, leaf count `C-S+1`, `B>=S` and `B<C`. A pre-activation claim cannot acquire a fabricated MMR proof.

The new public domain is Keccak-256 of `vara/gear-eth-bridge-domain/v2 || sourceDomain32 || destinationChainId32BE || originalQueue20`. Genesis identity is pinned separately. Python `hashlib.sha3_256` is not Ethereum Keccak. Do not copy the isolated Hoodi domain/client/Safe.

## 6. Ordered execution with explicit stop conditions

Only a later, separately authorized operator may execute this sequence. Each gate records finalized source/witness and EL/Beacon pins, exact commands, artifacts, reviewed proposal bytes and original transaction/message identities. Reconcile a handed-off intent before retrying; no automatic new signature, replacement, fee bump, receipt import or root reanchor.

### G0 — inventory, authority and rehearsal; no assets paused yet

- Resolve all advertised/unknown/unavailable contract/program identities, source-bytecode correspondence, full asset/fee/native custody inventory and old manager/token migrations.
- Obtain origin-complete ledgers and original worker journals. No private journal found is a blocker, not an empty queue.
- Reproduce current ledger/supply/residuals and cross-chain skew using matching effects. Confirm administrative history and all pending control messages.
- Review the real migration release, storage layout, public activation/key/weight plan, deployment/recovery authority and public-network fork support. Pin candidate artifacts but do not deploy them under this request.
- Rehearse on a fork/archive copy of the **actual public state**, including large history, every asset, reserved/partial receipt state, retired owners, an old outstanding redemption and all stored roots/nonces. Exercise genuine proofs, failure/restart and the exact authority route. Do not replace finality/proof validity with mocked success.

**Exit:** independently reviewed evidence, a real initializer ABI and deployable public activation/recovery proposal. Otherwise the migration remains BLOCKED and the existing lane is not altered by this document.

### G1 — freeze new asset admission while preserving recovery

1. Submit the reviewed ingress pause through real governance. Wait for the original EVM receipt to become canonically finalized, then independently observe `ERC20Manager.paused()==true`. Record EL stop watermark `E_stop` and include every lock/burn log through its finalized pause boundary.
2. Pause non-governance source bridge queueing through its authorized runtime route. Observe the finalized source block/hash, `Paused`, next global message nonce and queue state on both nodes. Record asset watermark `G_asset` and **next nonce** `N_asset`; all old successful asset queue messages must have nonce `< N_asset`.
3. Leave existing receipt/refund recovery possible while draining already-issued operations. Gear manager pause blocks both receipt submission and interrupted-transfer handling, so do not pause it prematurely and then declare a stalled receipt absent. Source-pallet pause prevents successful new non-governance queue insertion; any already-started burn/lock that now definitively fails needs its real refund and finalized accounting.
4. After inbound and interrupted operations are settled and no asynchronous request/payment effect is uncertain, finalize each active Gear manager's pause. Observe replies, `is_paused`, remaining tracker and balances at pinned hashes. Retired manager state still needs historical closure.

Source governance queueing remains permitted. UI/worker stopping alone is not an on-chain admission fence. Native token wrapping/unwrapping and fees remain separately inventoried; do not blindly pause a VFT/native exchange and strand users' backing.

**Exit:** no new successful asset admission, all older inbound effects fully processed, no ambiguous/reserved/partial effect or unresolved refund. The tool rejects an unresolved inbound cut rather than requiring replay-state import.

### G2 — close legacy proofs, registered redemptions and control history

- Run the legacy proof publisher only to reconcile original work. Register a valid ZK root for **every** unreleased old outbound claim and the specific reviewed pause/upgrade controls. Save original root block, root, timestamp, inclusion proof and verifier proof before the switch. Do not use legacy registration to preauthorize future ordinary resumes or unguarded rollback.
- Prefer completing releases now. If a valid registered claim remains, it may cross the cut only as a fully documented outstanding liability with preserved custody and an immediately usable original inclusion proof. The new verifier is not a way to recreate missing old roots.
- Reconcile original EVM signed transactions under their original sender/nonce/hash. Pending publication identities stay immutable. A late old-ZK publication after cutover must fail without modifying a root; it is not replaced by a BEEFY signature for the same intent.
- Reconcile paid/unpaid/missing request pairs, failed replies, tracker entries, and every old governance message. No obsolete upgrade, unpause, manager-addition or mapping-change message may remain able to execute unexpectedly. An already queued ordinary unpause must be canonically settled before the final freeze or canceled by an actually enforceable on-chain mechanism; a promise not to process its public proof is not a fence. If neither is possible, HOLD.
- Build a replay continuity index: all reconstructed old message nonces with `isProcessed`, registered roots/timestamps and proof digests; all original receipt keys/status/progress/floors per actual consumer; retired export/import/exit evidence. Preserve it permanently.

**Exit:** every old asset claim is released/refunded or redeemable from an already-stored root. Missing roots/proofs, unknown custody, effect ambiguity or unresolved control history means HOLD before verifier replacement.

### G3 — prepare the bounded source/EL cuts and avoid governance deadlock

- Propose a **future, fixed** source block ceiling `G_legacy` and new root floor `G_legacy+1`. Allow enough reviewed time for required pause/upgrade controls, not future resume messages, to be enqueued at or before the ceiling; do not extend it after an ambiguous handoff. The initializer proposal therefore has concrete boundary bytes before it is signed.
- Queue and register **only the exact reviewed upgrade** (and any still-needed pause controls) through the old lane. Retain their original nonces, payload hashes, source inclusions and ZK root proofs; wait the upgrade’s natural one-hour admin maturity. **Do not sign, queue or register ordinary resume/unpause or unguarded rollback messages before postchecks.** A matured governance message can be processed by anyone using its public proof; queue pause does not impose a postcheck predicate. Fresh resume messages are authorized only after G4/G5 and published through the qualified new BEEFY path, which remains available while source/user processing is paused.
- Pause ordinary queue processing through the reviewed pauser route after the desired drain. Observe finalized `paused()==true`; governance processing remains possible. Root publication itself is not paused, so verify all legacy publishers/journals and enforce the release's single-verifier boundary.
- Wait for `G_legacy` and `E_stop` to be finalized; capture the whole state again. Require no asset message after `G_asset`, all queued governance roots at or below `G_legacy`, no unaccounted late effects, and an unchanged deadline/intent. The global nonce may advance for governance; it must never reset.
- Keep new BEEFY publication disabled for source roots below the new floor. Verify the public client has independently accepted real commitments with adequate live coverage and the first allowed root is provable from retained MMR history. Verify the old queue's genesis/max-block/distance limits can accept the new range without a bypass.

**Exit:** immutable final cut package with both asset/control watermarks, stopped-user snapshots, original proofs/identities, exact reconciled per-asset accounting, qualified new BEEFY publication and **no executable preauthorized ordinary resume or unguarded rollback**. If the preselected ceiling is missed, hold and reconcile the attempted proposal; do not silently resign it with new bytes. A prestaged fallback is admissible only as a real separately reviewed **on-chain-conditional** action whose predicates enforce the required state; an ordinary unpause payload or operational promise does not qualify.

### G4 — atomic in-place upgrade; still held

- Independently revalidate the frozen package, authorities, code hashes, original verifier and initializer calldata at the fresh pinned finalized blocks. Run the offline consistency/proposal check; its result is not approval.
- Use the **original queued governance message** and stored root/inclusion proof to invoke `processMessage`. The queue calls GovernanceAdmin, which calls the original proxy's `upgradeToAndCall(newImplementation, initializerCalldata)` in one EVM transaction. No separate unguarded `setVerifier` transaction exists.
- Retain original EVM sender/nonce/hash/receipt and source governance message identity. A mined receipt is progress, not finalized cutover acceptance. If handoff is ambiguous, reconcile the original identity; never automatically submit a second upgrade.
- Wait for canonical finalized execution and verify proxy implementation/code hash, exact new verifier/client identity and enforced root floor. A revert leaves the prior transaction effects reverted, but that conclusion must be observed from the original receipt; transport failure is not evidence of rollback.

**Exit:** finalized atomic switch with users still held and all pre-cut replay/custody/maturity state unchanged. Any mismatch: HOLD, preserve journals and use the reviewed recovery/repair path, not a reset.

### G5 — continuity proofs before reopening

At common finalized pins, compare the post-state to the frozen package:

- Same queue/manager and Gear/token IDs; same original bindings, roles, source genesis, asset mappings and historical authorized manager set. No added receipt consumer or retired actor revival.
- Every old stored root and original timestamp unchanged; every previously true processed nonce still true. The upgrade governance nonce may have become true, so expect monotonic additions, not identical global state after execution.
- Same original receipt keys/statuses/progress/floors, checkpoint/endpoints and old-manager closure. No pair imported as processed to conceal a mint or reservation.
- Same per-asset supplies, custody and user balances, except explicitly matched, approved, finalized control/native-gas effects. Repeat the per-asset equations; no balancing by inference.
- Verify the retained old registered claim’s original inclusion, maturity, custody and processed-nonce fence against the frozen package and actual-state rehearsal; it must remain redeemable exactly once when ordinary processing is safely reopened at G6. Verify original processed receipt state/progress and replay behavior in the reviewed actual-state rehearsal; the real application rejection is exercised only after its consumer is safely reopened. Do not claim live redemption or `AlreadyProcessed` acceptance while the relevant queue/manager is paused.
- Genuine first post-floor BEEFY root/progress verifies from real source/witness commitments while ordinary processing remains paused. Exercise old-ZK and wrong-domain/queue/source rejection at the new root-verification boundary without accounting effects; do not deliberately invoke a conflicting valid root against public custody. Consumer nonce/receipt replay and typed application-rejection acceptance belong to G6’s controlled reopening or the genuine actual-state rehearsal, not a generic `Paused` rejection counted as replay proof.
- Root publication, inbound/checkpoint, outbound/payment workers resume from their original history and signed identities with one writer per signer. Cursor advancement is not proof that earlier work was processed.

G5 keeps ordinary user processing paused. Its checks establish structural/replay continuity and genuine new root-publication liveness, not a fictitious successful user redemption through a paused queue. Real old-claim redemption and application replay checks occur during the controlled G6 reopening, before new asset admission. If the new BEEFY publication path cannot carry a fresh governance root, remain HOLD; do not solve that failure by preauthorizing an unconditional resume. If an asset/network supports no safe minimum-value probe, keep it held; do not use mocked proof validity or infer readiness from another token.

### G6 — staged reopening and retirement

1. **Only after G4/G5 are independently verified**, authorize and queue a fresh queue-unpause governance message. Source governance queueing is permitted while paused, and root registration is not blocked by queue pause: publish its new source root through the already-qualified BEEFY path. Retain the fresh source/message/EVM identities, wait the pauser message’s natural 300-second maturity, then process it and observe canonical finalized unpause. This is not an old pre-registered resume message. Let documented old claims redeem with their original maturity and unchanged global nonce protection; observe the actual one-time release/replay outcome before reopening asset ingress.
2. Keep deposits/burn admissions paused while reconciling this reopening's actual old-claim releases and accounting. Retire only the old root-signing/publication owner, not its archive/proofs/journals or stored roots.
3. After that staged redemption accounting and independent review, create a **newly authorized** ERC20-ingress resume proposal and the authorized source/Gear-manager unpause operations. Queue and prove the EVM governance message through the new BEEFY path; retain its fresh original identity and normal maturity. Verify all effective origins, exact program replies and canonical finality. Exercise receipt replay rejection only under this reviewed controlled manager reopening, not while it is paused. Start no duplicate root owner or signer.
4. Perform separately authorized real per-asset roundtrips, receipt/nonce replay rejection, authority handovers and supervised restart under public timing/size/fees. Test native VARA and every inventoried origin separately; the four isolated test tokens do not cover them.
5. Retain old proof availability and redemption support until every old liability is finalized closed. Do not delete a legacy program/proof store because its publisher is no longer selected.

**Completion:** all old liabilities closed or continuously redeemable, all replay fences preserved, each asset's accounting explained, real public consensus/governance/recovery gates passed, and independently reviewed end-to-end evidence. This runbook currently has none of those execution approvals.

## 7. Rollback and incident boundaries

| Boundary | Permitted response | Forbidden response |
|---|---|---|
| Before public activation/queue switch | Cancel only proposals the actual governance system can cancel; reconcile already-queued messages. Resume the unchanged old lane only after verifying its exact state and liabilities. | Pretending an uncancellable queued message disappeared; deleting an intent or assuming failed transport means no effect. |
| Public session/domain/MMR activation | Follow the approved runtime recovery process while retaining all historical source state/nonces and real keys. | Re-genesis, queue-reset migration, reverting chain history or placeholder keys presented as real signatures. |
| Upgrade intent signed or handed off | Reconcile original sender/nonce/hash and governance message. Keep assets held until canonical outcome. | Resign/reanchor/rebroadcast a different identity to skip uncertainty. |
| Queue upgrade finalized, before new BEEFY proof | A separately reviewed reverse migration may be considered only if layout, replay state, old proof coverage and authority remain compatible. It is not automatically safe. | Reinstalling an older implementation that cannot read the new journal/layout; changing a verifier pointer without its guarded initializer. |
| First BEEFY root, new asset effect or irreversible program exit | HOLD and forward repair/recovery with complete original ledgers. Historical roots remain usable if safe. | Rolling back processed nonces, supplies, roots, session keys, old-program exit or finalized chain history. |

Root conflict/emergency is handled by the existing authority and challenge/recovery semantics, not a plan-specific exemption. Queue pause prevents normal user releases; choose the safest authorized state for already-proven redemptions rather than leaving them stranded by an indefinite operational pause. Missing independent recovery authority means public migration stays BLOCKED, regardless of the local Safe's threshold.

## 8. Runnable read-only checks and unsigned proposal package

### Existing read-only surface

Use the selected inventory's exact RPC, addresses and hash pins. Variables below are public parameters obtained from that record, not wallet keys. Do not read a signer `.env` to set them.

```sh
python3 tools/zk-migration-audit.py --help
python3 tools/zk-migration-audit.py docs/zk-to-beefy-inventory.json --deployment mainnet
python3 tools/zk-migration-audit.py docs/zk-to-beefy-inventory.json --deployment publicHoodi
```

With no separately reviewed plan these commands intentionally return exit **2**, `manifestStatus: BLOCKED`, `executionAuthorized: false`. They do not query RPC or write state.

Examples for an authorized read-only operator after resolving those public parameters:

```sh
cast call "$QUEUE" 'verifier()(address)' --rpc-url "$EL_RPC" --block "$EL_FINALIZED_HASH"
cast call "$QUEUE" 'paused()(bool)' --rpc-url "$EL_RPC" --block "$EL_FINALIZED_HASH"
cast call "$QUEUE" 'isProcessed(uint256)(bool)' "$ORIGINAL_NONCE" --rpc-url "$EL_RPC" --block "$EL_FINALIZED_HASH"
cast call "$MANAGER" 'messageQueue()(address)' --rpc-url "$EL_RPC" --block "$EL_FINALIZED_HASH"
cast call "$MANAGER" 'vftManagers()(bytes32[])' --rpc-url "$EL_RPC" --block "$EL_FINALIZED_HASH"
cast call "$MANAGER" 'tokens()(address[])' --rpc-url "$EL_RPC" --block "$EL_FINALIZED_HASH"
cast call "$TOKEN" 'balanceOf(address)(uint256)' "$MANAGER" --rpc-url "$EL_RPC" --block "$EL_FINALIZED_HASH"
cast call "$TOKEN" 'totalSupply()(uint256)' --rpc-url "$EL_RPC" --block "$EL_FINALIZED_HASH"
cast call "$TOKEN" 'decimals()(uint8)' --rpc-url "$EL_RPC" --block "$EL_FINALIZED_HASH"
cast code "$IMPLEMENTATION" --rpc-url "$EL_RPC" --block "$EL_FINALIZED_HASH"
cast storage "$QUEUE" 0x360894a13ba1a3210667c828492db98dca3e2076cc3735a920a3ca505d382bbc --rpc-url "$EL_RPC" --block "$EL_FINALIZED_HASH"
```

The last command reads the standard EIP-1967 implementation slot; the returned word is an address only after validating the proxy's actual mechanism and code. Hash code with `cast keccak`, not SHA3-256. Verify getters against the deployed ABI; a revert records unavailability, never a zero value. Do not use `--block finalized` separately for a multi-field gate because that tag can advance between calls.

For Gear, pin `chain_getFinalizedHead`, confirm `chain_getBlockHash(height)` with an independent archive witness, and use the existing read-only `gear_calculateReplyForHandle` at that hash. Its exact argument order is:

```text
[public_origin32, program32, SCALE_payload_hex, gas_limit_u64, value_u128, finalized_hash32]
```

Encode the service/method from the deployed IDL and fully decode the reply/code without trailing bytes. Query manager mapping, receipt status, tracker, admins, pause/config and historical proxy; query VFT balance/total supply/metadata and every role; query checkpoint/replay progress. The existing `vft-manager-tool read-transactions --block-number … --vft-manager …` is height-pinned, but its merged keys still require status queries. Existing `vft-tool … status` queries move and are diagnostics only. `checkpoints-tool` defaults to deployment; **do not run it for inspection**. No signer is required for `gear_calculateReplyForHandle`.

### Package consumed by `tools/zk-migration-audit.py`

The tool uses Python stdlib and installed `cast` only for offline Keccak/calldata encoding. It never calls RPC, signs, broadcasts, loads environment credentials or writes files. It reads only the explicitly named public JSON inventory/plan/evidence artifacts. Never put private keys or raw signed transaction bodies in that package.

The reviewed plan is a real operator-created JSON artifact with these concrete fields, not a supplied dummy ready-to-execute example:

| Field | Required contents |
|---|---|
| `deployment` | Exactly `mainnet` or `publicHoodi`. |
| `inventorySha256` | SHA-256 of the exact inventory bytes. |
| `bindings` | Each required name below maps to a JSON pointer to a provenance-bearing **measured** datum under `/deployments/<selected>/…`, or a field inside that datum’s `value` (for example `/deployments/mainnet/snapshots/source/value/genesisHash`). A projection keeps its enclosing datum’s provenance/snapshot; a nearer unknown/advertised datum cannot inherit measured status, and metadata fields are not measured values. No pointer may escape to the other deployment. |
| `preserve` | Exact original `queue`, `manager`, `gearGenesis`, ordered historical `vftManagers`, and `replayContinuitySha256` equal to the pinned replay artifact digest. These are values, not replacement addresses. |
| `cutover` | Nonnegative integer `assetFreezeBlock`, `legacyRootCeiling`, `newRootMinimum`, `nextNonceAtAssetFreeze`, `executionStopBlock`; `newRootMinimum=legacyRootCeiling+1`, within uint32 source range and finalized pins. |
| `assets` | Complete active one-to-one asset mappings covering exactly the measured `erc20Tokens` registry. Each row maps the names below to measured inventory pointers, and supplies `escrowParts=[{owner, value}]`, where `value` is a measured balance pointer. Ethereum-origin parts must cover exactly the original EVM manager; Gear-origin parts must cover every original authorized VFT-manager ID, including retired IDs whose balances are queried on the retained active token. Unknown balances hold, never become zero. Historical retired token mappings/custody remain in replay/ledger evidence, not silently discarded. |
| `artifacts` | Each required artifact maps to `{path, sha256}`. Paths resolve relative to the plan; files must be public JSON. |
| `initializer` | Audited real ABI `signature`, ordered `argumentBindings` and exact `calldata`. Bindings refer to the inventory names or `cutover.<field>`, and include `oldVerifier`, `candidateVerifier`, `cutover.legacyRootCeiling`. No initializer is guessed. |

Required binding names (also printed by importing the tool's constants if building a reviewed package):

```text
elChainId elGenesis elFinalizedHash elFinalizedNumber
gearGenesis gearFinalizedHash gearFinalizedNumber ethereumDiscoveryStartBlock
queue queueImplementation queueImplementationCodeHash oldVerifier oldVerifierCodeHash
manager managerQueue queueAdmin queuePauser managerAdmin managerPauser
adminSource pauserSource bridgeAdmin bridgePauser vftManagers erc20Tokens
queuePaused managerPaused sourcePaused
candidateSourceGenesis candidateDestinationChainId candidateQueue candidateVerifier
candidateSourceDomain candidateBridgeDomain candidateMmrStartBlock
candidateLatestBeefyBlock candidateMMRRoot candidateLive
releaseImplementation releaseCodeHash
```

Asset row names:

```text
erc20 gearToken tokenType supplyType decimalsEth decimalsGear consumer
gearMinter gearBurner gearAdmin gearPauser wrappedSupplyRaw
```

The live enum values must agree: EVM `TokenType.Ethereum=1` / Gear `TokenSupply.Ethereum=0`, or EVM `Gear=2` / Gear `Gear=1`. Match decimals and existing consumer IDs; do not map a token symbol instead of an address/program. The candidate's source genesis and destination queue/chain must equal the legacy custody lane. The public candidate must have a real accepted nonzero live commitment able to prove the first post-cut root.

Required public evidence artifacts:

| Name | Contents and independent review requirement |
|---|---|
| `authority` | Actual role/code/queue/source bindings and executable root/proxy/multisig/referendum route, with finalized pins, signatories/threshold/delays and filters. |
| `storageLayout` | Deployed/current/candidate layouts and compiler/OZ namespace compatibility; complete retained root/timestamp/processed-nonce/pause/role state. |
| `sourceActivation` | Real public runtime/code/genesis/domain/activation/MMR/key/validator/weight/finality/recovery evidence; old queue/nonce preservation. |
| `replayContinuity` | Original nonces, roots/timestamps, receipts/statuses/progress/floors and retired program export/import/exit chains; expected monotonic post-state comparisons. |
| `oldProofIndex` | Original old-root proofs and claim inclusion proofs with availability, immutable digests and original maturity. |
| `workerJournalIndex` | Original worker intents, non-secret identity/digest references, cursor completeness and outstanding original signatures/receipts. Never raw signed bytes. |
| `rehearsal` | Actual-state fork/archive rehearsal results, genuine proof/replay/failure/restart/continuity evidence and exact commands. |
| `releaseAbi` | Actual audited migration release ABI JSON (ABI array or an object with `abi`); initializer matches its exact signature. |
| `ledger` | Both directions and all controls, origin coverage and canonical per-effect claims under the exact original custody/replay namespace. |
| `accounting` | All assets at the same frozen cuts, raw-unit equations and independently explained surplus, native/fee backing and cumulative delta evidence. |

The `ledger` artifact supplies `gearGenesis`, `queue`, `manager`, `coverage` with `completeFromOrigin`, `ethereumStartBlock`, `ethereumThroughBlock` and `sourceThroughBlock`, an explicit `unresolved` array, and `claims`. Each claim has canonical `evidence`, `direction`, original `key` and `status`:

- `ethToGear`: `key=[EL_chain_id, manager20, original_tx_hash32, log_index]`, original `receiptKey=[slot,index]`, `consumer`, status `processed`. Multiple logs share one receipt key/consumer; two consumers for the same receipt hold. No reserved or partial receipt can pass.
- `gearToEth`: original `key=[source_genesis32, global_message_nonce]`, status `released` or `registered`; a `registered` claim additionally retains `root`, `rootTimestamp`, `proofArtifact`. Its nonce must be below `nextNonceAtAssetFreeze`. A `refundedNoQueue` request uses its original source/message identity with proof that no queue claim exists, not a manufactured queue nonce.

The `accounting` artifact has exactly one row per active asset pair, keyed by `erc20` and `gearToken`. `escrowRaw`, `wrappedSupplyRaw`, `inboundPendingRaw`, `outboundPendingRaw` and `provenSurplusRaw` are decimal integer **strings**; escrow/supply use the appropriate side for the asset origin in section 3. The first two must equal the measured inventory supply and the sum of the required custody observations; balancing numbers cannot overwrite observations. Rows satisfy the equation individually; inbound pending must be zero. Nonzero surplus requires `surplusEvidence` and independent historical review. A shape-valid `completeFromOrigin: true`, `measured` label, evidence string or file hash is **not** proof of completeness or validity.

Run only after that real package exists:

```sh
python3 tools/zk-migration-audit.py docs/zk-to-beefy-inventory.json \
  --deployment mainnet --plan "$REVIEWED_PUBLIC_PLAN"
```

Missing/inconsistent inputs return exit **2** and BLOCKED, with the immediate precise blocker. On a consistent package, exit **0** means only `CONSISTENT_FOR_UNSIGNED_REVIEW_ONLY`: output still sets `executionAuthorized:false` and migration readiness **BLOCKED pending independent finalized revalidation and governance approval**. It outputs only pre-cutover pause/upgrade proposals and an `upgradeToAndCall` simulation object, not signed transactions, an executable custody transfer or ordinary unpause proposals. These are not one batch. Fresh resume authorization must be created and queued only after finalized G4/G5 checks, through the qualified new BEEFY path; never sign, queue or publish an ordinary unpause or unguarded rollback beforehand.

The upgrade simulation object can be used in a separately authorized pinned `eth_call`/fork rehearsal with its recorded `from`, `to`, `data` and zero value. That simulation impersonates the existing GovernanceAdmin for validation only; it does not give an operator its private authority or replace the source governance/inclusion/maturity proof. Never send it with `cast send`.

## 9. Deliverable versus execution

This deliverable supplies a conditional, capability-grounded cutover, concrete gates, input/evidence formats, read-only checks and unsigned proposal preparation. It intentionally cannot claim present migration readiness: complete historical liabilities and retired migrations, reconciled USDT/WETH/VARA residuals, actual public authority/recovery, audited migration ABI/layout, public activation/real keys/weights and independently verified proof/finality/rehearsal evidence are still required.

No public bridge state, custody, signers, worker journals, deployment manifest, source runtime, Solidity ABI or program WASM was changed by preparing this plan. Any subsequent implementation or live migration needs its own approval and must preserve all original signed identities and historical evidence.
