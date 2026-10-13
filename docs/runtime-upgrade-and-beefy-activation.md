# Vara runtime upgrade and BEEFY activation

**Mainnet status: preparation only; executionAuthorized: false.** Mainnet needs separately approved production bytes, full-state rehearsal, validator readiness, governance and an asset-preserving verifier cutover. Authorization for a fresh Hoodi/local-Vara test does not authorize mainnet writes.

This guide describes the native v2.1.0 source protocol. The branch also contains an [existing-queue migration candidate](../ethereum/README.md#mainnet-custody-preserving-migration-candidate); a fresh test deployment does not qualify that migration or its retained liabilities.

## 1. Native v2.1.0 contract and qualification boundary

| Item | Required distinction |
|---|---|
| Exercised source checkout | User-provided v2.1.0 workspace at `f28e38e8c44885f4c58ba925efa204e190dea2ce`; rebuilt node identifies `2.1.0-f28e38e8c44` |
| Local artifact | Embedded `dev` runtime, `vara-testnet/20100`; not an approved mainnet production Wasm |
| Session ownership | `SessionKeys` runtime API v2 and native `author_rotateKeysWithOwner`; all five keys plus owner-bound proof |
| Destination | Root-only `GearEthBridge.bind_destination(chain_id: H256, queue: H160)`; immutable binding while paused |
| Source identity | Actual block-zero genesis, not a separately generated source-domain value |
| Cadence | 3000-ms slots and 2400-slot/two-hour epochs; no accelerated runtime |
| Capacity | Native runtime supports up to 1000 authorities; this Ethereum bridge still admits at most 256 and caps both signature paths at 86 |
| Mainnet readiness | **HOLD** until exact production-artifact, predecessor/full-state/weight, real-key, accounting and end-to-end gates pass |

Record the final production source and artifact independently. Earlier PR revisions, CI merge artifacts, reviews and custom fixture runtimes remain historical evidence, not interchangeable approvals. A matching version number does not prove identical runtime bytes or features.

## 2. Separate the approvals

1. **Release qualification:** approve the final source, production build, exact artifact and full-state rehearsal evidence.
2. **Inactive runtime installation:** authorize only the reviewed runtime-code proposal through the target chain's real Root governance route.
3. **Validator key installation:** authorize native owner-bound generation and registration; retain old private keystore entries until their actual current/queued use ends.
4. **Destination binding and BEEFY activation:** authorize the separate native Root calls in section 6 after their respective preconditions pass.
5. **Bridge operation/cutover:** separately qualify and authorize Ethereum contracts, actors, relayers and traffic under the migration owner.

Installing the runtime does not activate BEEFY. Setting its genesis block does not prove that a valid signed commitment has been produced. A signed commitment does not switch the Ethereum verifier or authorize token traffic. No activation-only second runtime upgrade is planned.

A disposable normal-cadence rehearsal needs distinct identities and authorization for its actual chain actions. Prove these transitions, authority handovers, restart catch-up and token/application effects; an uninterrupted timed soak is not required. Preserve original signed intents and operation deadlines through offline periods. Test acceptance is not public execution authority.

## 3. Release dossier and node readiness

Before any proposal, retain the following in the existing release/approval evidence owner:

- **Network:** target chain, independently approved source genesis, recent trusted GRANDPA/finality anchor, source and independent witness endpoints, and a common finalized pre-upgrade block/hash/state root. Two matching RPC responses alone do not establish independent finality or operator independence.
- **Artifacts:** final source commit, dependency/lockfile revisions, Rust/toolchain and build flags/features, compatible node binary digest, production Wasm path/size/SHA256, runtime metadata and version. Record hashes of the actual `:code` bytes separately and label their algorithms; do not equate a compressed artifact SHA256 with a differently encoded runtime-code hash.
- **Governance:** actual Root-producing route, submitter/authority, preimage/proposal identity, scheduling and cancellation rules, independent reviewer approval, exact decoded call and encoded-call digest. Do not assume mainnet Sudo, a Root-capable signer, a referendum track number or an available cancellation route.
- **State:** complete session keys and key owners, current/queued/eligible validator identities, BEEFY bookkeeping, GRANDPA/BABE state, bridge initialization/queue/root/nonce/pause/domain and pending cleanup/overflow state. Preserve original outstanding messages and relayer journals.
- **Qualification:** authenticated full-state migration and idempotency results, actual production-Wasm execution on a disposable state copy, reference-hardware weights/proof sizes and actual-roster capacity costs. The extra-read/proof-size manual allowance is not a completed weight benchmark.
- **Lane:** actual source genesis, destination chain ID, original MessageQueue proxy and computed bridge domain. The client/script field named `sourceDomain` must equal source genesis under this native protocol. Do not borrow a prior fixture lane, Safe, keys or genesis.

Deploy and qualify compatible node software **before the first upgraded block**, preserving chain identity, database and existing keystores. Keep BABE/GRANDPA running. Validators need BEEFY networking and signing support; proof-serving source/witness nodes need MMR data from its first insertion.

The existing node exposes `--enable-offchain-indexing true`. Proof-serving archives also need `--state-pruning archive --blocks-pruning archive` and durable offchain storage. `--offchain-worker` is not a substitute for indexing. Do not blindly replace existing service arguments, expose unsafe RPC publicly, purge a database or re-genesis a chain. Keep sufficient independent archival/proof-serving capacity through node restarts.

Authenticate the first MMR insertion from actual runtime history, independently of upgrade and BEEFY activation. Enable archival/offchain indexing before the first relevant block: later configuration cannot manufacture earlier MMR nodes. Missing history requires verified recovery/re-execution, not an invented start block.

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

The instrumented Wasm must enable `try-runtime`. **It is not the production Wasm and must not be proposed on-chain.** Record both digests and their source/build relationship. Separately execute the exact non-instrumented production bytes on the disposable authenticated state copy, including post-upgrade blocks and the native binding/activation calls; passing instrumented checks alone is insufficient.

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

## 5. Native owner-bound keys before activation

Migration placeholders are bookkeeping, not signing keys or possession evidence. Reject them even if their bytes decode as a curve point. Authenticate the exact v2.1.0 predecessor/layout and migration implementation rather than assuming an earlier prototype has the same contract.

For every current, queued and eligible validator:

1. Resolve the actual authorized session-key owner from finalized staking/session state. A legacy controller name or a stash label is not proof of the current owner. Retain enough unbonded balance for registration fees.
2. On that validator’s protected node RPC, call `author_rotateKeysWithOwner` with the raw 32-byte owner encoded as hex, not SS58 or a SCALE-length-prefixed vector. Require SessionKeys API v2, the native 161-byte five-key bundle and 321-byte proof. Check the returned bundle is present in that node’s keystore. Never log/export private seeds.
3. Submit `Session.set_keys(keys, proof)` through that owner using current metadata. The native proof authenticates all five keys: `babe`, `grandpa`, `im_online`, `authority_discovery`, then `beefy`. Do not splice a manually generated BEEFY key into the old four-key bundle or submit an empty proof. Preserve old private entries while current/queued sessions still require them.
4. Persist the exact original signed extrinsic before submission; reconcile its finalized dispatch and full `Session.NextKeys` readback on source and witness. The loopback-only `beefy-deploy source-keys --authority alice|bob` helper uses an immortal original for durable local-test recovery; this is not a mainnet operator signing policy. An expired historical mortal original remains evidence, not permission for automatic re-signing.
5. Observe real session transitions and reconcile current/next BEEFY sets, `Session.QueuedKeys`, `Session.NextKeys`, ownership and eligible entrants. Registration is not immediate activation. Native generation also changes GRANDPA keys; account for its existing delayed queue rollover. On a fresh normal-cadence local chain, initialization may require the new keys to become active across two session transitions. Never replace observations with a fixed sleep or shorten epochs.
6. Require supported nonempty sets, valid compressed 33-byte BEEFY points, unique keys/derived EVM addresses, expected ordered membership/set IDs and a roster within the bridge’s 256 ceiling. Native small committees require their real quorum, not an assumed large-set approximation.

Recheck near enactment and monitor pending governance. A node’s `author_hasSessionKeys` response is operator evidence, not independent cryptographic proof; genuine signed commitments and handovers remain required. Do not weaken quorum or activate a subset to accommodate missing operators.

BEEFY-only key changes preserve the original bridge queue. Actual GRANDPA authority changes retain the existing delayed rollover; follow GRANDPA events even when a queue does not roll over. While a clear is pending, `BridgeCleanupRequired` rejects every enqueue, including governance. This is not permission to reset the queue or fabricate an unsent/refunded message.

## 6. Bind the destination and activate through native calls

Compute and independently approve:

```text
bridgeDomain = keccak256(
  ASCII("vara/gear-eth-bridge-domain/v2")
  || actualSourceGenesis32
  || destinationChainId32BE
  || originalMessageQueue20
)
```

Use raw fixed-width bytes and Ethereum Keccak-256, not dynamic ABI encoding, ASCII hex, little-endian chain ID or SHA3-256. The source derives genesis itself from block zero. For a fresh destination, reserve and verify the deployer nonce/address prediction; for mainnet migration, retain the existing approved queue proxy.

### Bind while paused

Require a common finalized upgraded pin, paused bridge, absent `DestinationBinding` and zero `BridgeDomain`. Through the real Root route, dispatch the metadata-encoded native call:

```text
GearEthBridge.bind_destination(chain_id = H256(uint256_BE(destinationChainId)), queue = originalMessageQueue)
```

Client spelling: `api.tx.gearEthBridge.bindDestination(chainId32BE, queue20)`. The runtime enforces Root, pause, nonzero inputs and one-time binding. It stores `DestinationBinding = (actualSourceGenesis, chainId32BE, queue20)` and the computed domain. Read both back at the original finalized block on source and witness; retain the actual first binding block `D`. An already identical binding is reconciled, not submitted again. A conflicting binding is a HOLD. **Never patch `BridgeDomain` through raw storage or custom genesis.**

### Schedule BEEFY separately

After the independent release/key/readiness gates, require `Beefy.GenesisBlock = None` and no conflicting pending governance. Dispatch as Root:

```text
Beefy.set_new_genesis(delay_in_blocks = reviewedPositiveDelay)
```

Client spelling: `api.tx.beefy.setNewGenesis(delay)`. With actual execution block `A`, genesis is `G = A + delay`, not submission/proposal height. Check arithmetic bounds and retain that execution pin. The call can reset an already active genesis: **never use it as an idempotent retry**.

Binding is Normal dispatch; activation is Operational and carries a large declared weight. A mixed `Utility.batchAll` becomes Normal and was rejected by the native local runtime as `ExhaustsResources`. Preflight each actual governance wrapper’s native declared weight against its dispatch-class limits; submit these independently authorized calls separately. Do not understate weights with `sudoUncheckedWeight`, enlarge block limits, or reuse the obsolete raw-storage atomic-batch recipe. Mainnet uses its real reviewed governance route, not an assumed Sudo key.

Neither binding nor activation unpauses the bridge, initializes a fresh queue, or installs the Ethereum verifier. Persist each original signed intent and finalized inner-dispatch result. An uncertain outcome requires reconciliation, not another signature, nonce, binding or activation.

## 7. Prove activation; do not infer it from submission

Retain each original governance call’s successful outer/inner dispatch and finalized inclusion. At the corresponding common source/witness pins read:

| Surface | Required observation |
|---|---|
| `DestinationBinding` and `BridgeDomain` | Exact native genesis/chain/queue tuple and recomputed destination-bound domain |
| `Beefy.GenesisBlock` / `BeefyApi_beefy_genesis` | `Some(A + delay)` for the original activation; no fabricated block-zero discovery |
| `Beefy.Authorities`, `NextAuthorities`, `ValidatorSetId`, `SetIdSession` | Expected real ordered sets and session mappings, consistent with the key dossier |
| `BeefyApi_validator_set`, `BeefyMmrApi_authority_set_proof`, `BeefyMmrApi_next_authority_set_proof` | Authenticated current/next identities and roots; reject unsupported or empty state |
| `Mmr.RootHash`, `NumberOfLeaves` / `MmrApi_mmr_root`, `MmrApi_mmr_leaf_count` | Preserved, growing MMR with independently established start `S`, not reset to `G` |
| Original bridge storage | Queue, nonce, history, initialization and pause state preserved, with legitimate block/session changes reconciled |

For runtime API reads, use `state_call(method, "0x", finalizedBlockHash)` for the no-argument methods above and decode their exact SCALE return types, including `Option`/`Result`, against the approved runtime. For storage use `state_getStorage(key, finalizedBlockHash)` and authenticate proofs/state roots; default or absent values are not evidence of readiness. Do not mix latest/unfinalized state with pinned observations.

Activation-call finality proves scheduling only. After `G`, independently authenticate actual BEEFY signed commitments with native quorum and the correct sets. An unsigned MMR digest is not a signed BEEFY commitment. Require a signed post-binding checkpoint before Ethereum bootstrap; never substitute a zero/fabricated MMR root.

Prove source geometry and domain admission from archival history:

- Keep upgrade boundary, first MMR insertion `S`, BEEFY genesis `G`, domain binding `D` and later Ethereum root floor separate. Authenticate the first-leaf boundary; a leaf-count-derived candidate start alone is not sufficient evidence.
- For application snapshot block `B`, insertion is `L=B+1`; commitment block is `C`. Require `B<C`, `B>=S`, `leafIndex=B+1-S` and `leafCount=C-S+1`, with checked arithmetic and the actual authenticated count/root.
- First admissible application snapshot is at least `max(S, D, newRootFloor)` when the destination floor applies. A valid first leaf commits parent `S-1`; it does not authorize that parent's application history or an earlier domain.
- Generate and verify native MMR proofs against the signed commitment, including queue identity, 86-byte snapshot commitment, timestamp and destination binding. Wrong-domain, pre-admission, malformed and trailing-byte cases must reject.
- Demonstrate two authenticated authority handovers, restart catch-up, six-asset settlement and paid-application effects on the normal-cadence test lane. Preserve the original operation deadlines; reconnecting does not renew them. A successful activation call alone is not acceptance, and no uninterrupted soak is required.

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

After activation, do not re-genesis, reset the MMR/bridge queue, clear nonces, change a live domain or downgrade to an incompatible runtime. Successful binding/activation has no automatic rollback. Corrective governance needs its own reviewed state-preserving procedure.

The operator handoff must contain separate terminal verdicts for **artifact qualification**, **inactive runtime installation**, **real-key readiness**, **domain/BEEFY activation**, **normal-Hoodi acceptance** and **public bridge cutover readiness**. Each verdict includes command exits, artifact hashes, exact proposal/call identities, original transaction references, finalized source/witness pins, state/proof evidence and unresolved conditions. None is predeclared successful by this document.

## Source references and verification boundary

Runtime source exercised for the native local integration: `f28e38e8c44885f4c58ba925efa204e190dea2ce` in the user-provided Gear checkout. Review its `vara/runtime/vara/src/lib.rs`, session migrations, bridge leaf encoding, `vara/pallets/gear-eth-bridge/src/lib.rs`, native session-key author RPC and pinned SDK BEEFY dispatch before selecting production artifacts. The native API/metadata and exact executable/Wasm hashes are separate evidence from a release label.

See [local nodes and Hoodi operation](running-the-bridge.md#local-nodes-and-hoodi) and the [custody-preserving migration gates](../ethereum/README.md#mainnet-custody-preserving-migration-candidate).

Local verification rebuilt the node with its real embedded development Wasm, checked matching source/witness code and metadata, observed empty-proof `Session.InvalidProof`, and finalized native owner-bound registration with identical-original recovery. This is not production-Wasm/full-state qualification, mainnet key readiness, public transfer acceptance, or governance approval.

The fresh native-local-Vara/public-Hoodi lane subsequently finalized 1 GOT outbound and 1 USDC inbound, with exact recipient, escrow and supply effects, both indexers committed, and the actual frontend showing both Completed. The follower recovered on an archival HTTPS endpoint without replacing its 27 original signed commitments; a separate read-only history audit authenticated all 27 with no pending records or original-journal writes. Explicit mainnet execution-network admission and canonical execution finality were also exercised read-only. This qualifies the fresh test lane and network boundary, not a retained-state mainnet migration.

The later public-source refresh at block 36,889,688 still reported Vara spec 11000, 59 validators, four-field queued session keys and no BEEFY/MMR runtime APIs at the same hash on both endpoints. These provider observations do not replace authenticated source finality, production-artifact approval, validator readiness or governance evidence. The mainnet bootstrap, liability reconciliation and exact-profile cutover gates remain open.
