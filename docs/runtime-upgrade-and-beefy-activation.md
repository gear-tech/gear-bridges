# Vara runtime upgrade and BEEFY activation

**Status: preparation only; executionAuthorized: false.** This runbook prepares one runtime upgrade followed by a separately authorized BEEFY activation. It does not authorize a production build, key rotation, governance submission, signature, broadcast, activation or Ethereum verifier change. No deployment-specific proposal bytes are supplied: the approved production artifact, target-state pins, authority route and lane parameters are still prerequisites.

The public custody/verifier cutover remains owned by [ZK-to-BEEFY migration](zk-to-beefy-migration.md). This guide covers the source runtime only; it neither implements nor clears that document's G0–G6 gates. Historical source observations in that document must not be used instead of the source revision and activation contract below.

## 1. Approved source and current decision

| Item | Recorded value / decision |
|---|---|
| Runtime PR | [gear-tech/gear #5642](https://github.com/gear-tech/gear/pull/5642) |
| Approved source revision | `19ab81dd208b3ce7b339f5d1dbd144511d3c7e2c` in sibling `runtime-production`; never substitute sibling `source` or fast-runtime PR #5644 |
| Revision change | Supersedes `f961bed815dd4ab0802703620605ea3b3659ac60` only by the optional empty-Unix-matrix CI guard; runtime Rust is unchanged |
| PR CI | [37550611143](https://github.com/gear-tech/gear/actions/runs/37550611143), attempt 1: success, 24 successful jobs and 5 skipped |
| Actual CI workflow revision | PR merge revision `702810496fe0e04a8008909491615b01848ad707`; do not label these artifacts as exact approved-head production Wasm |
| GitHub status observed during preparation | `OPEN`, `REVIEW_REQUIRED`, `BLOCKED`; the earlier implementation-review waiver is not release approval |
| Runtime version in the approved source | Mainnet `spec_name=vara`, `spec_version=20100`; `dev` changes the name to `vara-testnet` and enables different runtime configuration |
| Supported four-key predecessor | Matching `spec_name`, `System.LastRuntimeUpgrade.spec_version=11000`, exact supported session-key storage layout |
| Cadence | 3000-ms slots, 2400-slot/two-hour epochs; no fast-cadence change |
| Bridge capacity | Retain 256 authorities and both EVM signature ceilings of 86; independently qualify the actual current/next roster |
| Readiness | **HOLD:** independent review, exact production-artifact/full-state/weight qualification, real key readiness and normal-Hoodi acceptance are not complete |

If review or merge changes the source revision, approve and record the new pin before building or proposing it. Do not silently use the merge commit, a moving branch or an older compiled binary. Preserve all prior evidence and retained campaign identities/deadlines.

The runtime node README's older reference to 1000-validator qualification is superseded by the approved 256-cap decision. A synthetic 59-validator test is not evidence of the public roster. At N=59 the expected native quorum is 40 and EVM sampling requires 20 distinct signatures; at N=256 these are 171 and 86 respectively.

## 2. Separate the approvals

1. **Release qualification:** approve the final source, production build, exact artifact and full-state rehearsal evidence.
2. **Inactive runtime installation:** authorize only the reviewed runtime-code proposal through the target chain's real Root governance route.
3. **Validator key installation:** authorize each operator's BEEFY key provisioning and session registration; preserve the four existing keys.
4. **Domain binding and BEEFY activation:** authorize the one atomic Root batch in section 6 only after installation and readiness checks pass.
5. **Bridge operation/cutover:** separately qualify and authorize Ethereum contracts, actors, relayers and traffic under the existing migration owner.

Installing the runtime does not activate BEEFY. Setting its genesis block does not prove that a valid signed commitment has been produced. A signed commitment does not switch the Ethereum verifier or authorize token traffic. No activation-only second runtime upgrade is planned.

A disposable normal-cadence rehearsal needs its own explicit test authorization and distinct identities. It must run these transitions before proving the five-hour/two-handover token and paid-application campaign. Public migration implementation remains gated on that acceptance; test acceptance is not public execution authority.

## 3. Release dossier and node readiness

Before any proposal, retain the following in the existing release/approval evidence owner:

- **Network:** target chain, independently approved source genesis, recent trusted GRANDPA/finality anchor, source and independent witness endpoints, and a common finalized pre-upgrade block/hash/state root. Two matching RPC responses alone do not establish independent finality or operator independence.
- **Artifacts:** final source commit, dependency/lockfile revisions, Rust/toolchain and build flags/features, compatible node binary digest, production Wasm path/size/SHA256, runtime metadata and version. Record hashes of the actual `:code` bytes separately and label their algorithms; do not equate a compressed artifact SHA256 with a differently encoded runtime-code hash.
- **Governance:** actual Root-producing route, submitter/authority, preimage/proposal identity, scheduling and cancellation rules, independent reviewer approval, exact decoded call and encoded-call digest. Do not assume mainnet Sudo, a Root-capable signer, a referendum track number or an available cancellation route.
- **State:** complete session keys and key owners, current/queued/eligible validator identities, BEEFY bookkeeping, GRANDPA/BABE state, bridge initialization/queue/root/nonce/pause/domain and pending cleanup/overflow state. Preserve original outstanding messages and relayer journals.
- **Qualification:** authenticated full-state migration and idempotency results, actual production-Wasm execution on a disposable state copy, reference-hardware weights/proof sizes and actual-roster capacity costs. The extra-read/proof-size manual allowance is not a completed weight benchmark.
- **Lane:** approved `sourceDomain32`, destination chain ID, original MessageQueue proxy address and resulting bridge domain. Source genesis and source domain are separate identities. Do not borrow the retained Hoodi lane's domain, Safe, keys or genesis.

Deploy and qualify compatible node software **before the first upgraded block**, preserving chain identity, database and existing keystores. Keep BABE/GRANDPA running. Validators need BEEFY networking and signing support; proof-serving source/witness nodes need MMR data from its first insertion.

The existing node exposes `--enable-offchain-indexing true`. Proof-serving archives also need `--state-pruning archive --blocks-pruning archive` and durable offchain storage. `--offchain-worker` is not a substitute for indexing. Do not blindly replace existing service arguments, expose unsafe RPC publicly, purge a database or re-genesis a chain. Keep sufficient independent archival/proof-serving capacity through node restarts.

MMR insertion starts with the upgraded runtime, **before** BEEFY activation. Turning indexing on only at activation cannot manufacture earlier offchain MMR nodes. Missing history requires a verified archival recovery/re-execution path before proceeding, not a fabricated start block.

## 4. Full-state rehearsal and inactive upgrade

### Rehearse before proposing

Authenticate the selected snapshot against the approved finalized header/state root and independent witness. Capture the whole state, including child tries; a pallet-filtered snapshot is not the release gate. Prove the supported predecessor using live runtime identity **and** stored upgrade metadata, not the repository version alone.

The installed `try-runtime-core 0.10.1` supports these commands. They are instructions for a later approved rehearsal; they were not executed against chain state when preparing this guide. Supply reviewed public endpoints, an authenticated block hash and fresh output paths; never load signer credentials for this work.

```sh
# Run only after the snapshot/rehearsal inputs are approved.
set -e
: "${SOURCE_RPC:?approved archive endpoint required}"
: "${PRE_UPGRADE_HASH:?authenticated finalized block hash required}"
: "${SNAPSHOT:?fresh snapshot path required}"
: "${TRY_RUNTIME_WASM:?reviewed instrumented Wasm required}"
test ! -e "$SNAPSHOT"
try-runtime create-snapshot --uri "$SOURCE_RPC" --at "$PRE_UPGRADE_HASH" \
  --child-tree "$SNAPSHOT"
try-runtime --runtime "$TRY_RUNTIME_WASM" on-runtime-upgrade \
  --checks all --disable-mbm-checks --blocktime 3000 snap --path "$SNAPSHOT"
```

The instrumented Wasm must enable `try-runtime`. **It is not the production Wasm and must not be proposed on-chain.** Record both digests and their source/build relationship. Separately execute the exact non-instrumented production bytes on the disposable authenticated state copy, including post-upgrade blocks and the eventual activation batch; passing instrumented checks alone is insufficient.

Keep spec-name, spec-version and idempotency checks enabled. `--disable-mbm-checks` is the documented exception for this single-block migration: multi-block simulation fabricates predecessor metadata that the migration correctly rejects. Do not weaken the runtime guard or fabricate `LastRuntimeUpgrade` to make a rehearsal pass. A predecessor/layout mismatch can halt runtime execution; it is a release blocker, not a recoverable skipped record.

Verify the complete migration tuple, not only session conversion:

- `MigrateSessionKeys` preserves the original four keys, queued ordering and ownership, adds deterministic BEEFY placeholders, and initializes only missing inactive BEEFY bookkeeping. Existing records are not reset. The intended first activation requires BEEFY to remain inactive.
- The configured bridge `set_hash` migration updates the GRANDPA authority-set hash; the destructive bridge `reset` migration is **not** part of this tuple. Never add it or invoke it as an activation shortcut.
- Non-`dev` configuration also establishes the bridge builtin's ED lock where needed; verify the treasury transfer and lock outcome. Scheduler pause-task migration is also in the tuple. Account for these intentional changes in the reviewed state diff.
- Bridge queue, global message nonce, initialization, retained roots/history, pause state and domain survive except for explicitly reviewed existing behavior. In the isolated migration comparison there must be no unexplained loss. Across real blocks, reconcile legitimate message/session changes rather than demanding an impossible unchanged full state root.
- Re-running migrations is idempotent; malformed or unsupported session records fail the release check. Test delayed activation and BEEFY-only key changes with nonempty original bridge state.

### Propose only the qualified code

The intended inner Root call is **`System.set_code(code = approved production Wasm bytes)`** (`api.tx.system.setCode(code)` in metadata-driven clients). Use the pre-upgrade runtime metadata to encode the upgrade call and the actual chain's reviewed governance/preimage route to dispatch it as Root. Preimage submission and proposal submission are themselves public writes and require authorization.

Do not use `set_code_without_checks`, raw writes to `:code`, an instrumented/dev Wasm or a combined upgrade-and-activation proposal. Decode the final call independently and compare its code bytes with the approved artifact before handoff.

After authorized enactment, retain the original extrinsic/proposal identity, dispatch result and finalized `System.CodeUpdated` evidence. Then wait for finalized execution under the new runtime; code installation in a block is not proof that the next block's migrations succeeded. Source and witness must agree on:

- installed `:code` bytes/digests, metadata, `spec_name`, version and normal cadence;
- completed migrations and all preserved-state invariants above;
- `Beefy.GenesisBlock = None`, unless an independently approved different starting state invalidates this first-activation procedure;
- MMR growth and the authenticated first-insertion boundary `S`, with usable archive/offchain proofs.

Any discrepancy means HOLD. Installing the inactive runtime does not authorize automatically proceeding to section 6.

## 5. Real BEEFY keys before activation

The migration's placeholder is derived as `0x02 || keccak256(validator AccountId bytes)`. It is bookkeeping, not proof of a valid signing key or validator possession. Reject placeholders even if their bytes happen to decode as a curve point.

For every current, queued and eligible validator:

1. Provision a real secp256k1 ECDSA BEEFY key in the correct validator node's protected keystore, key type **`beef`**. Obtain operator possession evidence without exporting seeds/private keys. A protected local `author_hasKey(publicKey, "beef")` observation is useful operator evidence, not independent cryptographic proof by itself.
2. Preserve existing `babe`, `grandpa`, `im_online` and `authority_discovery` public keys. The upgraded `SessionKeys` adds `beefy` last. Do not blindly call `author_rotateKeys` and replace every consensus key to add BEEFY.
3. Through the actual authorized session-key owner, register **`Session.set_keys(keys, proof)`** using the upgraded metadata. Record original signed transaction identity and finalized outcome. An accepted call/ownership-proof field alone is not sufficient evidence of real BEEFY key possession.
4. Observe session transitions until **current and next BEEFY authority lists**, current validator identities, `Session.QueuedKeys`, registered `Session.NextKeys` and key ownership agree at one common finalized pin. A `NextKeys` write is not immediate authority activation. Do not substitute a fixed sleep or an assumed number of sessions for these observations.
5. Validate nonempty supported sets, valid compressed 33-byte curve points, unique source keys and derived EVM addresses, expected ordered membership/set IDs and the independently authenticated actual roster within the bridge's 256 ceiling. Reconcile eligible validators that can enter before enactment.

Repeat readiness checks close to the approved enactment boundary and monitor changes while governance is pending. Stop on missing/duplicate/placeholder keys, unavailable signers, changed membership or mismatched current/next roots. Do not weaken quorum or activate a subset to accommodate missing keys.

BEEFY-only key changes preserve the original bridge queue. Actual GRANDPA authority changes retain the existing delayed rollover; follow GRANDPA events even when a queue does not roll over. While a clear is pending, `BridgeCleanupRequired` rejects every enqueue, including governance. This is not permission to reset the queue or fabricate an unsent/refunded message.

## 6. Bind the lane and activate atomically

### Freeze the domain and preconditions

Compute, independently cross-check and approve:

```text
bridgeDomain = keccak256(
  ASCII("vara/gear-eth-bridge-domain/v2")
  || sourceDomain32
  || destinationChainId32BE
  || originalMessageQueue20
)
```

Concatenate raw fixed-width bytes, not ABI dynamic encoding, ASCII hex or little-endian chain ID. Use Ethereum Keccak-256, not SHA3-256. Never use the source genesis as an implicit substitute for the approved source domain.

At a common finalized upgraded pin require all release/key gates, `Beefy.GenesisBlock = None`, and an unbound domain or the identical independently approved binding with its original historical binding block. Reject a conflicting nonzero domain. Never rebind a live lane to another destination.

Resolve and independently decode the storage entry from the approved upgraded metadata. At the pinned source the raw key is:

```text
GearEthBridge.BridgeDomain
Twox128("GearEthBridge") || Twox128("BridgeDomain")
0xfd6e027f7a1bd8baa6406cea4d80d93263f4028887270c25d6e5b18d5b9cbc6b
```

Its value is SCALE `H256`: **exactly the computed 32 bytes**, with no compact-length prefix or option tag. The outer `set_storage` call's key/value vectors still have their normal metadata-driven SCALE encoding. Do not write `sourceDomain32` directly into this entry.

### Prepare the single Root call

Using the upgraded metadata, independently encode/decode this call tree and retain its digest in the approved governance proposal:

```text
Utility.batch_all(calls = [
  System.set_storage(items = [(GearEthBridge.BridgeDomain raw key, bridgeDomain raw32)]),
  Beefy.set_new_genesis(delay_in_blocks = 1)
])
```

Client spellings are `api.tx.utility.batchAll`, `api.tx.system.setStorage` and `api.tx.beefy.setNewGenesis(1)`. The **outer batch must dispatch as Root** through the established governance route. Do not use `Utility.batch`, submit the calls separately, include an unpause or change any other storage key.

**Atomic is not compare-and-set.** The SDK `set_new_genesis` checks Root and delay >= 1, but permits resetting an already active BEEFY genesis; `set_storage` has no expected-old-value predicate. Neither call enforces real key readiness or domain immutability. These are trusted-governance preconditions, not claimed runtime protections. Ensure no conflicting pending Root action can invalidate them before enactment; if this cannot be established, HOLD. Do not introduce an unreviewed guard pallet or promise a second runtime upgrade.

With execution block `A` and delay 1, the call stores **`G = A + 1`**; it does not use the proposal/submission block. Reject a schedule near the block-number arithmetic bound. Record the actual finalized execution height, not an estimated governance date. `D` is the actual first approved domain-binding block, normally `A` for a new binding; retain an earlier authenticated `D` for an identical pre-existing binding.

Before retrying an uncertain submission, reconcile the original proposal/extrinsic against finalized state. A second successful call would move `G`; it is not an idempotent retry. Never alter an original signed intent, nonce or journal to obtain another activation attempt.

## 7. Prove activation; do not infer it from submission

Retain the original batch's successful governance/inner dispatch result and finalized inclusion. At the same block hash on source and witness read:

| Surface | Required observation |
|---|---|
| `GearEthBridge.BridgeDomain` | Exact approved 32-byte destination-bound value |
| `Beefy.GenesisBlock` / `BeefyApi_beefy_genesis` | `Some(A + 1)` for this batch; no fabricated block-zero discovery |
| `Beefy.Authorities`, `NextAuthorities`, `ValidatorSetId`, `SetIdSession` | Expected real ordered sets and session mappings, consistent with the key dossier |
| `BeefyApi_validator_set`, `BeefyMmrApi_authority_set_proof`, `BeefyMmrApi_next_authority_set_proof` | Authenticated current/next identities and roots; reject unsupported or empty state |
| `Mmr.RootHash`, `NumberOfLeaves` / `MmrApi_mmr_root`, `MmrApi_mmr_leaf_count` | Preserved, growing MMR with independently established start `S`, not reset to `G` |
| Original bridge storage | Queue, nonce, history, initialization and pause state preserved, with legitimate block/session changes reconciled |

For runtime API reads, use `state_call(method, "0x", finalizedBlockHash)` for the no-argument methods above and decode their exact SCALE return types, including `Option`/`Result`, against the approved runtime. For storage use `state_getStorage(key, finalizedBlockHash)` and authenticate proofs/state roots; default or absent values are not evidence of readiness. Do not mix latest/unfinalized state with pinned observations.

The batch's finality is only the scheduling proof. Wait until `G` is reached and independently authenticate actual BEEFY signed commitments with native quorum and the correct current/next sets. An unsigned MMR digest exists while BEEFY is inactive and is **not** a signed BEEFY commitment.

Prove source geometry and domain admission from archival history:

- Keep upgrade boundary, first MMR insertion `S`, BEEFY genesis `G`, domain binding `D` and later Ethereum root floor separate. Authenticate the first-leaf boundary; a leaf-count-derived candidate start alone is not sufficient evidence.
- For application snapshot block `B`, insertion is `L=B+1`; commitment block is `C`. Require `B<C`, `B>=S`, `leafIndex=B+1-S` and `leafCount=C-S+1`, with checked arithmetic and the actual authenticated count/root.
- First admissible application snapshot is at least `max(S, D, newRootFloor)` when the destination floor applies. A valid first leaf commits parent `S-1`; it does not authorize that parent's application history or an earlier domain.
- Generate and verify native MMR proofs against the signed commitment, including queue identity, 86-byte snapshot commitment, timestamp and destination binding. Wrong-domain, pre-admission, malformed and trailing-byte cases must reject.
- Demonstrate two authenticated authority handovers and real restart recovery in the distinct normal-cadence test campaign. Keep its five-hour warmup, original 60-minute token-batch and 44-minute application limits. Prove six-asset and paid-application effects; a component seal or successful activation call is not acceptance.

Do not attach the corrected root-keyed slot-12 queue implementation to the retained block-keyed Hoodi queue. Ethereum verifier installation, custody, actor replacement, original claims and traffic remain under their separate migration/release gates.

## 8. Stop, reconcile and hand off

| Observation | Required response |
|---|---|
| Unsupported predecessor/layout, wrong artifact or incomplete full-state rehearsal | Do not propose the upgrade; resolve and re-review the release |
| Upgrade dispatch recorded but post-upgrade execution/finality unhealthy | HOLD activation; preserve diagnostics and use the separately approved runtime incident process |
| Placeholder/missing keys, changed queued set, absent MMR history or conflicting domain | Do not submit/enact activation; reconcile readiness and pending governance |
| `GenesisBlock` already set or original activation outcome unknown | Reconcile the original intent; never reset genesis as a retry |
| Domain/genesis correct but no valid signed commitments or handover gap | Keep bridge operation held; repair original node/key/proof availability without weaker verification or root injection |
| Conflicting source/witness state or unexplained queue/nonce changes | HOLD; preserve both observations and investigate before any subsequent write |

After activation, do not re-genesis, reset the MMR/bridge queue, clear nonces, change a live domain or downgrade to an incompatible runtime. Atomic rollback of a failed batch is not a rollback plan for a successful activation. Any corrective governance action needs its own reviewed state-preserving procedure.

The operator handoff must contain separate terminal verdicts for **artifact qualification**, **inactive runtime installation**, **real-key readiness**, **domain/BEEFY activation**, **normal-Hoodi acceptance** and **public bridge cutover readiness**. Each verdict includes command exits, artifact hashes, exact proposal/call identities, original transaction references, finalized source/witness pins, state/proof evidence and unresolved conditions. None is predeclared successful by this document.

## Source references and verification boundary

All runtime references below are pinned to the approved source, not a moving branch:

- [Node upgrade/activation procedure](https://github.com/gear-tech/gear/blob/19ab81dd208b3ce7b339f5d1dbd144511d3c7e2c/vara/node/README.md#beefy-upgrade-and-later-activation).
- [Runtime version, session keys, pallet configuration and APIs](https://github.com/gear-tech/gear/blob/19ab81dd208b3ce7b339f5d1dbd144511d3c7e2c/vara/runtime/vara/src/lib.rs).
- [Session migration and predecessor checks](https://github.com/gear-tech/gear/blob/19ab81dd208b3ce7b339f5d1dbd144511d3c7e2c/vara/runtime/vara/src/migrations/session_keys.rs) and [complete migration tuple](https://github.com/gear-tech/gear/blob/19ab81dd208b3ce7b339f5d1dbd144511d3c7e2c/vara/runtime/vara/src/migrations.rs).
- [Atomic batch and original-queue preservation test](https://github.com/gear-tech/gear/blob/19ab81dd208b3ce7b339f5d1dbd144511d3c7e2c/vara/runtime/vara/src/integration_tests.rs#L1345-L1421).
- [Bridge snapshot encoding](https://github.com/gear-tech/gear/blob/19ab81dd208b3ce7b339f5d1dbd144511d3c7e2c/vara/runtime/vara/src/bridge_leaf.rs) and [domain fixture](https://github.com/gear-tech/gear/blob/19ab81dd208b3ce7b339f5d1dbd144511d3c7e2c/vara/runtime/vara/tests/fixtures/bridge_commitment.json).
- [Pinned SDK BEEFY activation dispatch](https://github.com/paritytech/polkadot-sdk/blob/298f676c91d64f15f38ea7fd78f125c5889ab09c/substrate/frame/beefy/src/lib.rs#L276-L290) and [System calls](https://github.com/paritytech/polkadot-sdk/blob/298f676c91d64f15f38ea7fd78f125c5889ab09c/substrate/frame/system/src/lib.rs).
- [Shared operations and normal-runtime admission](running-the-bridge.md#isolated-hoodi-beefy-token-qualification).

Documentation preparation checked the installed CLI options, source call semantics and offline storage/domain encoding. It did not run the full-state rehearsal, build a runtime, provision keys, encode a deployment-specific governance proposal or execute any chain action. Earlier local tests and green PR CI remain separate evidence, not production upgrade qualification.
