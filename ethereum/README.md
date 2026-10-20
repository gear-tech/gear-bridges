## Foundry

**Foundry is a blazing fast, portable and modular toolkit for Ethereum application development written in Rust.**

Foundry consists of:

- **Forge**: Ethereum testing framework (like Truffle, Hardhat and DappTools).
- **Cast**: Swiss army knife for interacting with EVM smart contracts, sending transactions and getting chain data.
- **Anvil**: Local Ethereum node, akin to Ganache, Hardhat Network.
- **Chisel**: Fast, utilitarian, and verbose solidity REPL.

## Documentation

https://book.getfoundry.sh/

## Usage

### Install dependencies

```shell
$ forge soldeer install
```

### Build

```shell
$ forge build
```

### Test

```shell
$ forge test
```

### Format

```shell
$ forge fmt
```

### Gas Snapshots

```shell
$ forge snapshot
```

### Anvil

```shell
$ anvil
```

### Deploy

```shell
# deployment
$ cp .env.example .env
# protective retained-mainnet configuration (generic fresh deployment rejects its default mode)
$ cp .env.example.mainnet .env
# fresh Sepolia deployment configuration
$ cp .env.example.sepolia .env
# fresh Hoodi deployment configuration
$ cp .env.example.hoodi .env

$ source .env

# Simulate first; --broadcast requires separate approval for the exact action.
$ forge script script/Deployment.s.sol:DeploymentScript --rpc-url "$LOCAL_RPC_URL" -vvvv
$ forge script script/RetainedBridgeMigration.sol:RetainedBridgeMigrationScript --rpc-url "$RETAINED_RPC_URL" -vvvv
$ forge script script/Deployment.s.sol:DeploymentScript --rpc-url "$SEPOLIA_RPC_URL" -vvvv
$ forge script script/Deployment.s.sol:DeploymentScript --rpc-url "$HOODI_RPC_URL" -vvvv
```

### BEEFY deployment and regression checks

Every fresh entrypoint uses BEEFY: `DeploymentScript`, `BeefyLocal`, `BeefyHoodi`
and `BeefyTokens` share the existing full bridge environment and queue CREATE
predictions in `test/Base.sol`. The generic environment entrypoint requires
`DEPLOYMENT_MODE=fresh-beefy`; it rejects the protective
`retained-beefy-migration` default in `.env.example.mainnet`. Retained migration
uses the separately gated existing-proxy workflow below, never generic fresh deployment.
Its explicit entrypoint is
`forge script script/RetainedBridgeMigration.sol:RetainedBridgeMigrationScript --rpc-url "$RETAINED_RPC_URL"`.
Keep broadcast disabled for preparation/readback; a write requires the independently
approved stage/action and original-transaction reconciliation described below.


Before any broadcast, configure source governance/manager/emergency identities,
all required `BEEFY_*` bootstrap values in the network env example, and an existing
positive-delay `BEEFY_RECOVERY_TIMELOCK`. The bridge domain is
`keccak256(abi.encodePacked("vara/gear-eth-bridge-domain/v2", sourceDomain,
bytes32(chainId), predictedQueue))`. No missing bootstrap falls back to another
verifier. Bootstrap timestamps must be recent, and current/next validator IDs
must be consecutive with nonempty, at-most-256-member sets.

The client consumes one nonce. Generated-token profiles consume seven more
CREATEs, complete existing-token overrides consume three, and governance,
verifier and queue implementation consume four before the queue proxy. Thus queue
offsets are `+12` for generated tokens, `+8` for complete token overrides and `+5`
for message-only Hoodi. Partial, duplicate or code-less overrides fail before
deployment. Mainnet requires all four canonical assets; test-token generation
requires explicit `DEPLOY_TEST_TOKENS=true` on a supported test chain.
`BeefyTokens` also checks its expected starting nonce. The configured bridge domain
must match the predicted queue and chain; client/verifier bind that destination immutably.

Full-bridge behavioral tests retain tokens, governance, custody and ordinary fees.
Only their explicit `ForkBeefyFixture(..., true)` admits checkpoints through the
test-only `BeefyClientMock`; queue envelopes and MMR inclusion are still verified
by the real `VaraQueueRootVerifier`. Consensus, real deployment, migration,
recovery and transaction smoke use signed commitment admission.

The interactive and Fiat-Shamir submissions share authenticated leaf acceptance,
authority rotation, and ECDSA verification. Ticket handling, randomness, sampling,
proof-check ordering, signature counts, and usage counters remain protocol-specific.

Run the focused checks with installed dependencies/compiler caches:

```shell
$ forge test --offline --match-path test/BeefyInterop.t.sol
$ forge test --offline --match-path test/BeefySubmissionParity.t.sol
$ forge test --offline --match-path test/VaraBeefyQueue.t.sol
$ forge test --offline --threads 1 --match-path 'test/script/*.t.sol'
```

Deployment tests mutate process-wide environment variables, so run them with one
thread. If OpenZeppelin validation reports duplicate contracts in `out/build-info`,
use a fresh task-owned `FOUNDRY_OUT` and `FOUNDRY_CACHE_PATH` for that check rather
than deleting existing deployment/build evidence. A forced no-cache build followed
by selectively compiled tests can also leave duplicate build-info batches; run
the focused suite in one fresh output directory. Keep script smoke outputs
separate from test outputs to avoid combining incompatible build-info batches.

For a deployment dry-run, supply fresh bootstrap/domain environment values and run
`forge script --offline script/BeefyLocal.s.sol:BeefyLocal --chain-id 31337`
(or the matching Hoodi/token script and chain). Omit `--broadcast`; simulation
alone sends no transactions. Never retry a partial broadcast without reconciling
receipts, deployed contracts, and the deployer nonce.

These scripts deploy fresh, isolated BEEFY lanes; they do not migrate an existing
ZK queue proxy. Use the gated existing-proxy migration below instead.

### Full-transaction BEEFY gas and operating cost

Run from this directory with the existing Foundry tools and Python standard library:

```shell
python3 script/benchmark-full-transactions.py
```

The command owns one ephemeral loopback-only Anvil (chain 31337, explicit Osaka
hardfork and fixture-era timestamp), mines real transactions using Anvil's test
account, and stops only that node. It accepts no remote RPC or private key. It
reuses the existing compiler/dependencies and signed 59/150/256-validator
`beefy-interop.json` fixtures. The first update uses their saved signatures;
subsequent ordinary updates and handovers are genuinely ECDSA-signed synthetic
local source snapshots, using the same fixture keys/Merkle proofs and the
unchanged verifier. Handovers rotate through the real fixture authority roots
and lengths (59→150→59, 150→256→150, 256→59→256), rather than measuring only
same-root storage writes. The bridge/source domain remains the original client's
domain throughout each sequence. No proof verifier, expiry, quorum or custody
policy is bypassed. This is a local gas benchmark, not source-runtime activation
or production BEEFY certification.

The queue/token smoke deploys through the same `Base` environment and
`fresh-beefy` env validation, including all token registrations, governance and
ordinary payment setup. Its recovery timelock is deployed before queue prediction.
The runner supplies only its known disposable Anvil key and an owned-runner flag.


`out/fulltx-benchmark/receipts.json` retains actual RPC transactions (including
full calldata), successful mined receipts and inclusion headers, fixture/source
SHA-256 digests, tool/compiler settings and checked post-state. Forge broadcast
records and logs remain beside it; `--output <directory>` selects a separate
evidence location. Compiler artifacts and cache use project-relative
`out/fulltx-<hash>` and `cache/fulltx-<hash>`, keyed by the resolved evidence
directory. This isolates the smoke build from full-build artifacts while retaining
the existing filesystem allowlist and OpenZeppelin upgrade validation.
The bridge deployment invocation uses a clean compiler batch so OpenZeppelin
validation cannot consume overlapping build-info; fixture cases reuse the cache.
Existing test Anvils and public networks are not modified.
Do not run two benchmark writers against the same output directory/build cache.

Measured with Forge/Anvil 1.7.1, Solc 0.8.37, optimizer + via-IR, Osaka (2026-10-15):

| Signing validators | Selected signatures | First saved-fixture update | Ordinary update | Largest measured handover |
|---:|---:|---:|---:|---:|
| 59 | 20 | 361,162 | 345,430 | 450,484 |
| 150 | 51 | 876,546 | 849,160 | 939,465 |
| 256 | 86 | 1,421,390 | 1,409,880 | 1,514,546 |

Every number above is receipt `gasUsed`, **not** a `gasleft()` delta. In
particular the historical call-only 229,615 / 552,825 / 898,382 measurements are
not transaction costs. The largest handover rows include changing roots and
validator-count/counter-array transitions: 256→59→256, 150→150→256 and
150→256→150 respectively (previous current → signing set → newly announced
next set). They are maxima of these measured cases, not a protocol-wide gas
upper bound. Ordinary/handover snapshots use a single authenticated MMR leaf;
the first saved-fixture update has its fixture proof. Real source MMR depth,
payloads, key bytes, storage state and transitions can change the receipt cost.

| Transaction case | Calldata bytes | Standard intrinsic gas | EIP-7623 calldata floor | Actual receipt gas |
|---|---:|---:|---:|---:|
| 59 ordinary | 10,084 | 131,116 | 296,290 | 345,430 |
| 59 largest handover | 10,052 | 130,436 | 294,590 | 450,484 |
| 150 ordinary | 26,532 | 331,920 | 798,300 | 849,160 |
| 150 largest handover | 26,756 | 335,564 | 807,410 | 939,465 |
| 256 ordinary | 45,188 | 559,484 | 1,367,210 | 1,409,880 |
| 256 largest handover | 45,188 | 559,472 | 1,367,180 | 1,514,546 |
| Queue root publication | 740 | 27,020 | 36,050 | 161,983 |
| Escrow-backed ERC20 message execution | 484 | 23,860 | 28,150 | 133,365 |

For these non-creation transactions without access lists, let `T = zeroBytes +
4 × nonzeroBytes`. Standard intrinsic gas is `21,000 + 4T`; the post-Pectra
EIP-7623 floor is `21,000 + 10T`. The command counts the actual transaction
bytes and checks the floor against each receipt. None of the measured rows is
floor-bound, but the floor must not be omitted when comparing calldata-heavy
proof designs. Receipt gas already includes intrinsic gas and the applicable
floor/refund rules: **do not add either again**.

The root/execution rows use the existing signed queue fixture construction and
real queue/verifier/manager proxies plus CircleToken, not a verifier/handler
mock. The separate three-validator anchor costs 116,697 gas and is not a
59-validator update price. After the root's configured onchain delay, execution
moves exactly 10 units from 30 units of manager escrow, leaves 20 there, and
marks the original nonce processed. This measures an Ethereum-supply ERC20
release, not every token/native path. Previously reported Hoodi receipts
(174,605 root publication / 155,428 ERC20 execution) are separate historical
observations, not results of this local command or universal gas constants.

For a gross monthly budget before fee income, use explicit cadence and traffic:

```text
monthly_gas = days × (handovers_per_day × handover_gas
                    + additional_updates_per_day × ordinary_update_gas)
            + roots_per_month × root_gas
            + messages_per_month × message_gas
            + other_monthly_gas
monthly_USD = infrastructure_USD
            + monthly_gas × effective_gas_price_gwei × 1e-9 × ETH_USD
            + vara_execution_amount × VARA_USD + contingency_USD
```

Count additional updates separately from required ordered authority handovers;
multiple messages can share a root/checkpoint, but each execution still costs
gas. Include deployment amortization, failed originals/retries, governance and
other paid transactions in `other_monthly_gas` as applicable. Infrastructure
includes RPC/archive/source services and relayer/follower hosting. If transaction
classes have different effective prices, price and sum them separately.

At 30 days and an assumed 12 handovers/day, the conservative measured
59-validator handover scenario alone is **162,174,240 gas/month**. Add bounded
traffic-triggered updates, roots and executions; do not multiply the historical
call-only measurement by cadence. Neither 12/day nor a gas price/ETH exchange
rate is a production guarantee: use the actual source schedule and explicitly
label price assumptions (for example ETH_USD = 3,000 is a planning assumption,
not a current quote). For a US$5,000 gross ceiling, require the complete formula
to stay below that ceiling with operational headroom; fee income is not a
substitute for the gross-cost bound. Sparse scheduling does not waive any
source-domain, latest-anchor, authority-handover, original-signature, finality or
24-hour client-expiry requirement.

See the [operating envelope](../docs/running-the-bridge.md#monthly-operating-cost-envelope)
for the US$5,000 target, infrastructure allowances, traffic/fee conditions and
the separately identified, not-yet-deployed queue-root coalescing design.


### Expiry recovery and initializer scope

`initializeBeefy` keeps the five legacy setup arguments and adds the approved
`recoveryTimelock` address. Unlike `initialize`, it checks the actual proxy/chain
binding and pins the authenticated source publication floor. These are runtime
deployment invariants; immutable client policy remains checked once by the
verifier constructor, not repeated by the queue or after `new BeefyClient`.
Never use the legacy initializer to bypass BEEFY setup.

Set `BEEFY_RECOVERY_TIMELOCK` to an independently reviewed, deployed positive-delay
timelock controlled by the approved multisig, never a relay hot key. Verify its
code, multisig owners/threshold, proposer/canceller/executor/admin roles and delay.
`getMinDelay()` only checks a reported positive delay, not its enforcement, Safe
identity or governance quality. The manifest binds controller address, code hash
and delay; changed or missing evidence HOLDs.
The local-only `LOCAL_RECOVERY_PRIVATE_KEY` option creates an explicit development
timelock from a separate signer without changing the bridge deployer's nonce
geometry. It is not production governance qualification.

Recovery is trusted re-bootstrap authority: a compromised multisig can select
malicious validator history and forge bridge/governance messages. Monotonic
height/domain checks do not prove continuity with the expired validator set.

After client expiry, only that timelock may execute
`recoverBeefyVerifier(expectedVerifier, replacementVerifier, expectedMaxBlock)`.
The replacement must retain source identity, queue/chain binding and MMR history
start, be live with a nonzero authenticated root, and advance beyond both the
expired checkpoint and queue maximum. Recovery preserves the publication floor,
roots, maturity, custody and replay state and leaves claims paused. Unpause through
a newly authenticated source-governance message; emergency-stop controls remain.
Keep the candidate client live through the timelock delay. Requalify its bootstrap
and deployment manifest without discarding any original transaction journals.
This narrow recovery capability grants no UUPS upgrade or general pause role.


### Mainnet custody-preserving migration candidate

Keep the existing queue proxy, ERC20 manager, token contracts and Vara programs.
The queue at `0x8E01Fbf136cA97627ca241dB9EFf1DFE3f2195F6` used implementation
`0xadf9f599277f41907cf86109833d1601b433dc5b` (initializer version 6) at Ethereum
block 26,183,239. Its authenticated runtime bytecode and real claim inputs are in
`test/fixtures/mainnet-legacy-queue.json`; compilation used Solc 0.8.35, with only
compiler-declared implementation-address immutables substituted for comparison.

The candidate exposes `migrateToBeefy(expectedLegacyVerifier, beefyVerifier,
expectedLegacyMaxBlock, firstBeefyBlock, recoveryTimelock)` as an admin-only `reinitializer(8)`.
Execute it atomically through the existing authority's `upgradeToAndCall` while
paused. It preserves roles, custody, genesis, maximum progress, registered roots,
legacy root-keyed maturity and every replay bit, including blocked nonce 836.
It does not grant a replacement UUPS administrator; the separately reviewed
timelock receives only the expiry-recovery capability above. Expected verifier
and maximum are compare-and-swap guards: pausing claims does not stop publication.

The new floor must exceed the legacy maximum, be at or after MMR history start,
fit uint32 and remain within the existing 57,600-block progress bound. The
queue-bound, policy-compatible BEEFY client must be live and already have an
authenticated nonzero MMR root covering the floor. No old ZK proof is accepted
after migration. Previously registered roots remain redeemable below the floor;
new registrations use block-specific maturity. Reconcile and publish all needed
legacy root coverage before selecting the floor; catch up with genuine ZK roots
if the MMR start is beyond the distance bound. Never invent empty progress.

Run the bytecode transition regressions and the opt-in archival fork rehearsal:

```shell
forge test --match-contract VaraQueueRootVerifierTest --match-test testLegacyMigration
MAINNET_MIGRATION_RPC_URL="$ARCHIVE_RPC" forge test --match-contract VaraQueueRootVerifierTest --match-test testMainnetForkMigration
```

The fork pins block hash
`0x8e462035baf49ef0bdd7ede06fb133e6aca0f34c25833387d3c87500947a7a3d`, checks deployed
code/version, restricts queue migration writes to the implementation, initializer,
verifier, floor and recovery-timelock slots, preserves existing custody/supply and redeems authentic
pending claim 21 with its retained proof. It preserves replay bits 836 and 864 and
rejects repeated redemption. It uses local authority impersonation and fixture
BEEFY authorities, not an approved mainnet bootstrap or a source governance proof.
Without the RPC variable, the fork test is explicitly skipped, not qualified.

The shared fork path in `test/Base.sol::deployBridge` pins the mainnet block,
legacy code and initializer, retains all existing bridge contracts, and calls the
same typed migration validation, calldata and governance helpers as the operational
runner. Only pause and atomic-upgrade roots are registered under the old verifier.
The expected legacy maximum includes those two roots, not an unpause permission.
After the paused upgrade passes retained-state readback, a fixture-signed empty
BEEFY epoch advances the queue. Only then is a new unpause message constructed in
a later authenticated epoch; it must wait for its own governance maturity.

This remains a **fork rehearsal**, not production authorization. Synthetic legacy
source-proof checks are scoped mocks and immediately cleared. BEEFY signatures and
MMR proofs are real fixture-authority proofs. Finalized pins are explicitly local
fixture evidence; the operational runner instead checks canonical RPC blocks and
original transaction receipts. No source governance impersonation, replacement
administrator, token replacement or broadcast key is permitted in the fork path.

Run the integrated retained-state scenario explicitly:

```shell
REQUIRE_MAINNET_MIGRATION=true MAINNET_MIGRATION_RPC_URL="$ARCHIVE_RPC" forge test --match-contract MainnetBridgeMigrationTest
```

It preserves all five token addresses, custody/supplies, legacy maturity and replay
bits; redeems pending claim 21; settles a new BEEFY-backed claim only after its own
maturity; and verifies failed-governance rollback plus subsequent authorized
upgrade/unpause through BEEFY roots. Empty BEEFY proofs and the placeholder admin
are rejected. Mock-based unit suites always deploy fresh fixtures; the removed
`FORK_URL_OR_ALIAS`/governance-address environment switches no longer turn them into
migration tests. The dedicated CI job sets `REQUIRE_MAINNET_MIGRATION=true`, making
a missing archive RPC fail the integrated scenario rather than silently skip it.
An archive rejection is a failed qualification, not permission to change the pin.

#### Retained migration operator inputs

Use `script/RetainedBridgeMigration.sol:RetainedBridgeMigrationScript`, with
`DEPLOYMENT_MODE=retained-beefy-migration` and `RETAINED_RPC_URL` pointing to the
same chain as `--rpc-url`. The fresh deployment entrypoint deliberately rejects
this mode. Preparation, verification and reconciliation do not read a signing key.
No replacement queue, manager, token or governance authority is deployed.
Use Foundry v1.8.5, matching CI: v1.7.1 can compile the scripts but lacks their
required RPC cheatcode overload. Add `-vvvv` to preparation runs to inspect the
`RetainedMigrationPrepared` event containing the migration calldata and upgrade payload.


Keep approved inputs under `broadcast/migration-inputs/`, outside disposable
build output. Types are declared in
`script/RetainedBridgeMigration.sol`; encode the exact Solidity structs with
`abi.encode`, not packed encoding or JSON object field ordering:

| Input | JSON shape / approval |
|---|---|
| `RETAINED_PLAN` | `{"schemaVersion":1,"plan":"0x..."}` containing `MigrationPlan`; exact-file Keccak must equal independently approved `RETAINED_PLAN_HASH` |
| `RETAINED_ACTION` | `{"schemaVersion":2,"action":"0x..."}` containing `MigrationAction`; exact-file Keccak must equal `RETAINED_ACTION_HASH`; `action.planHash` binds the approved plan file; schema-1 actions are rejected |
| `RETAINED_SOURCE_CUT` | `queue`, `sourceDomain`, `legacyMax`, `firstBeefyBlock`, and `roots` containing ABI-encoded `LegacyRootPin[]`; file digest must equal `plan.sourceCutApproval` |
| `RETAINED_UNPAUSE_INVENTORY` | `queue`, `sourcePauser`, `unpauseNonces` array; digest must equal `plan.outstandingUnpauseInventory`; every listed old permission must already be consumed |
| `RETAINED_DEPLOYMENTS` | `implementation`, `client`, `verifier`, each containing ABI-encoded `OriginalTransaction`; exact-file digest must equal `RETAINED_DEPLOYMENTS_HASH` |
| `RETAINED_FINALITY_OUTPUT` | Absent output path for the stage-9 upgrade observation or stage-10 original-transaction reconciliation; capture reserves `<path>.capture` and writes the exact observation file |
| `RETAINED_UPGRADE_OBSERVATION` | Captured `{"schemaVersion":1,"planHash":"0x...","stage":8,"transactionHash":"0x...","observation":"0x..."}`; `observation` is ABI-encoded `FinalityObservation`, copied into `action.barrier.upgradeReadback`; exact-file Keccak must equal `RETAINED_UPGRADE_OBSERVATION_HASH` |
| `RETAINED_BARRIER_OBSERVATION` | Same captured format with numeric `stage:10` and the original empty-progress transaction; copy the decoded observation into `action.barrier.publication`; exact-file Keccak must equal `RETAINED_BARRIER_OBSERVATION_HASH` |

Review decoded fields before approving their file digests. A freshly calculated
hash is not independent approval. The plan pins the chain/genesis, proxy and
underlying implementation hashes, all existing contracts/source authorities,
token/payment inventories, cut/floor, constructor bootstrap, recovery delay and
roles, and retained-state readbacks. Include bridge custody, root/maturity,
replay and token-policy readbacks in `retainedState`; include applicable custom
proxy implementation slots in `retainedProxies`. These pins are checked at each
stage and against the archived upgrade poststate, so do not pin globally mutable
canonical-token `totalSupply` across stages: unrelated WETH wrapping or USDC
issuance must not invalidate a migration. Keep supply-preservation evidence
transaction-local (for example, an upgrade trace proving no token mint/burn),
separate from the immutable cross-stage pins; refreshing a supply pin would
conflict with the archived poststate. Custody, roles, configuration and inventory
checks remain mandatory. Supplied roots and nonce lists
must come from the complete independently approved liability/permission audit;
the tool cannot infer missing historical obligations from an incomplete list.
The RPC must support canonical finalized blocks, archival state and recovery
timelock grant/revoke history.
The recovery-role inventory must include every live role, including the
timelock's implicit `CANCELLER_ROLE` grants to proposers.

Each invocation handles one approved stage and at most one transaction:

| Stage ordinal | Action | Required predecessor |
|---|---|---|
| 0 | Prepare calldata and upgrade payload | Complete approved retained plan/cut/inventory |
| 1–3 | Deploy implementation, client, verifier | Exact planned signer/nonce/address; finalized original constructor records for prior deployments |
| 4 | Bootstrap | All three original constructors authenticated; real BEEFY signature-entrypoint calldata |
| 5–6 | Register pause and upgrade roots | Genuine legacy proofs; no root beyond the approved legacy ceiling |
| 7 | Pause | Both governance roots mature; every approved liability root registered; exact approved maximum |
| 8 | Atomic upgrade | Paused queue, live bootstrapped client, unchanged identities and complete registered cut coverage |
| 9 | Verify and capture upgrade finality observation | Original upgrade authenticated; canonical finalized paused poststate with complete cut coverage; absent `RETAINED_FINALITY_OUTPUT` |
| 10 | Publish empty BEEFY barrier | Independently approved upgrade observation; source timestamp later than its `observedAt` plus allowed clock skew |
| 11 | Register new unpause root | Original empty-barrier receipt and approved barrier observation; strictly later source epoch and timestamp |
| 12 | Process unpause | Original root-registration receipt authenticated and governance maturity passed |

Bootstrap can require multiple separately approved stage-4 actions for the
interactive signature path. It never bypasses client signature verification or
the normal follower's pre-migration binding checks.
Constructor history remains pinned to the original bootstrap inputs; legitimate
native authenticated validator handovers do not require rewriting that approval.

Every stage from Verify onward requires `action.upgrade`, containing the approved
upgrade delivery and original transaction evidence. Matching state produced by
another executor cannot substitute for an unresolved original transaction.
Complete approved liability-root coverage is rechecked after migration as well
as before Pause.

After stage 9, independently approve the captured file and its exact Keccak before
setting `RETAINED_UPGRADE_OBSERVATION[_HASH]` and approving a source empty epoch.
After stage 10 is finalized, reconcile its original transaction with a new absent
`RETAINED_FINALITY_OUTPUT`; approve that capture as
`RETAINED_BARRIER_OBSERVATION[_HASH]` before authorizing the later unpause epoch.
Re-encode and independently approve the schema-2 action with both observations.
An observation contains the qualified finalized `state`, the then-current latest
`head`, and `observedAt` in UTC seconds. Source timestamps must exceed the relevant
`observedAt * 1000 + MAX_FUTURE_SOURCE_SKEW_MS`, not the older inclusion timestamp.
`observedAt` is the later of the current host clock and observed head timestamp.
The qualified RPC and host clock are explicit operator trust boundaries; this is
**not a cryptographic proof of historical consensus-finality time**. The output's
parent directory must exist. Keep original observation files and their `.capture`
reservations; never overwrite them or discard a failed capture to bypass
reconciliation. An authorized retry after a failed capture needs a distinct
output path.

For a new write, `PRIVATE_KEY` must match the action's approved signer, and both
latest and pending nonce must equal its approved nonce. Set
`RETAINED_BROADCAST_ARTIFACT` to an absent destination before the first attempt.
After Foundry records the transaction, point it to the immutable original
timestamped artifact and supply `RETAINED_BROADCAST_HASH`,
`RETAINED_TRANSACTION_INDEX`, and `RETAINED_TRANSACTION_HASH` for reconciliation.
Do not use a mutable `run-latest.json` as retained evidence. Keep original files
unchanged; deployment-history records reference their exact hashes and indices.

An atomic marker under `broadcast/` owns the chain/signer/nonce before an actual
broadcast attempt. Missing receipts, pending inclusion, reverts, changed intent,
noncanonical blocks and rival successes all HOLD; never delete the marker or
blindly use `--resume` to create another signature. Reconciliation authenticates
the original transaction's input, value, signer, nonce, destination/creation,
receipt and canonical finalized inclusion before checking state.

Unpause also requires `RETAINED_ROOT_ARTIFACT`, `RETAINED_ROOT_ARTIFACT_HASH`,
`RETAINED_ROOT_TRANSACTION_INDEX` and `RETAINED_ROOT_TRANSACTION_HASH`.
Its barrier carries the original empty-progress transaction and approved finality
observations. Both source timestamps must exceed the relevant observed finality
time by the client's allowed future skew; a higher message nonce is insufficient.
Outstanding old unpause permissions remain a separate governance-reconciliation
gate, not something this script can safely cancel or ignore.

Release pipeline, in order:

1. Authenticate the exact legacy and candidate compiler inputs/artifacts and run
   OpenZeppelin storage validation, including namespaced base storage. Preserve
   the working branch and extract a reviewed migration candidate; do not merge
   the whole architectural WIP as a release shortcut.
2. Approve the production source runtime, weights/full-state migration rehearsal,
   validator BEEFY keys, destination-bound domain and activation/MMR insertion pins.
   At the 2026-10-13 read-only refresh, both public Vara endpoints reported spec
   11000 at block 36,889,688, without BEEFY/MMR runtime APIs or BEEFY fields in
   queued session keys. These are matching provider observations, not authenticated
   source finality or proof of production activation.
   Never wire destructive queue/nonce reset migration. Retain the original queue.
3. Qualify mainnet follower network admission, Ethereum execution/Beacon finality
   and immutable bootstrap evidence. Follower, root publisher and history audit now
   accept explicit `--ethereum-network mainnet` with a reviewed `mainnet-token-stack`
   manifest, chain 1 and the fixed Ethereum mainnet genesis. Hoodi remains the default;
   mainnet rejects fixture/rehearsal approval and non-TLS execution endpoints.
   Network support is not approval of production bootstrap or Beacon/source state.
   Preserve original journals; exercise dropped-original recovery, external progress
   and backfill against the approved retained-state profile.
   The normal token follower requires the queue to be bound to the new verifier,
   while migration requires an accepted nonzero client MMR root beyond the cut floor.
   Separately qualify the original bootstrap commitment before the atomic queue
   cutover; the normal follower is not a pre-migration binding-check bypass.
4. Reconcile all five assets, all historical managers/native programs, paid and
   unpaid pending messages, registered and missing roots, fees and replay state
   through an approved cut. Token identity agreement and sample claims are not
   complete liability reconciliation. Keeping old Vara managers does not apply
   this branch's newer manager code to those immutable programs.
5. Exercise both transfer directions, SDK/frontend settlement and HOLD behavior,
   both indexers, source governance authorization and interruption/restart against
   the exact retained-state release profile. A fresh-token deployment is not this
   acceptance test. Retain legacy claim services and historical proofs.
6. Review implementation/verifier/client hashes, existing governance authority,
   recovery timelock code/roles/delay and multisig policy, expected legacy
   verifier/maximum, floor, calldata and pause/unpause sequencing. Only separately
   authorized production governance may execute the upgrade. Rehearse expiry
   recovery without rolling back accepted progress, custody or processed claims.

Source activation, mainnet follower qualification, full accounting, end-to-end
acceptance and governance approval remain release gates. Local fork success
neither authorizes a public transaction nor qualifies mainnet activation.


### Copied BEEFY sources and licenses

`src/beefy/BeefyClient.sol` and the Snowfork utilities retain their upstream
copyright and SPDX headers. The interoperability source pin is Snowbridge
[`1201293e482ef052b9c3989dcf680046704fef3d`](https://github.com/Snowfork/snowbridge/tree/1201293e482ef052b9c3989dcf680046704fef3d);
the architectural reference is
[`630c2081359e2417496e7f4c1a189a7eb40f7bd4`](https://github.com/Snowfork/snowbridge/tree/630c2081359e2417496e7f4c1a189a7eb40f7bd4).
The local client and utilities are modified for Vara; no upstream audit is
transferred to this code or its deployment bytecode.

- [Apache-2.0 text](src/beefy/LICENSE-APACHE-2.0): verbatim Snowbridge
  `contracts/LICENSE` at the interoperability pin (identical at the architectural
  reference). Neither pinned upstream tree includes a `NOTICE` file.
- [MIT text](src/beefy/LICENSE-MIT): verbatim OpenZeppelin Contracts
  [`v5.0.0/LICENSE`](https://github.com/OpenZeppelin/openzeppelin-contracts/blob/v5.0.0/LICENSE),
  preserving its 2016–2023 copyright notice. `utils/Math.sol` also preserves
  the OpenZeppelin and Snowfork copyright notices from the copied source.
- [Original Bits MIT text](src/beefy/LICENSE-MIT-solidity-examples): verbatim
  [`ethereum/solidity-examples/LICENSE`](https://github.com/ethereum/solidity-examples/blob/ba15d56b62cf384e983006bb139af59a03ce22dc/LICENSE)
  at `ba15d56b62cf384e983006bb139af59a03ce22dc`, the upstream credited by
  `utils/Bits.sol`; the original copyright line is preserved as supplied.


### Relaying

Root production follows the existing commitment follower → MMR root publisher →
unchanged individual-message delivery pipeline. `SubmitMerkleRootScript` reads
`MMR_PROOF`, the canonical v2 queue-proof envelope (not raw sibling hashes).
The independent observer may challenge a confirmed historical-root mismatch;
governance response is separately authorized. The challenge pause is finite;
persistent incidents require the existing governance pause.


```shell
$ source .env

$ forge script script/relayers/ProcessMessage.s.sol:ProcessMessageScript --rpc-url $LOCAL_RPC_URL --broadcast -vvvv
$ forge script script/relayers/ProcessMessage.s.sol:ProcessMessageScript --rpc-url $MAINNET_RPC_URL --broadcast --verify -vvvv
$ forge script script/relayers/ProcessMessage.s.sol:ProcessMessageScript --rpc-url $SEPOLIA_RPC_URL --broadcast --verify -vvvv
$ forge script script/relayers/ProcessMessage.s.sol:ProcessMessageScript --rpc-url $HOODI_RPC_URL --broadcast --verify -vvvv

$ forge script script/relayers/SubmitMerkleRoot.s.sol:SubmitMerkleRootScript --rpc-url $LOCAL_RPC_URL --broadcast -vvvv
$ forge script script/relayers/SubmitMerkleRoot.s.sol:SubmitMerkleRootScript --rpc-url $MAINNET_RPC_URL --broadcast --verify -vvvv
$ forge script script/relayers/SubmitMerkleRoot.s.sol:SubmitMerkleRootScript --rpc-url $SEPOLIA_RPC_URL --broadcast --verify -vvvv
$ forge script script/relayers/SubmitMerkleRoot.s.sol:SubmitMerkleRootScript --rpc-url $HOODI_RPC_URL --broadcast --verify -vvvv
```

### Upgrade

> [!WARNING]
> Existing ZK queues use the reviewed `migrateToBeefy` calldata below, never a fresh initializer or an empty-data verifier cutover. Other proxy upgrades require their own reviewed initialization and storage validation.

```shell
$ source .env

$ forge script script/upgrades/ERC20Manager.s.sol:ERC20ManagerScript --rpc-url $LOCAL_RPC_URL --broadcast -vvvv
$ forge script script/upgrades/ERC20Manager.s.sol:ERC20ManagerScript --rpc-url $MAINNET_RPC_URL --broadcast --verify -vvvv
$ forge script script/upgrades/ERC20Manager.s.sol:ERC20ManagerScript --rpc-url $SEPOLIA_RPC_URL --broadcast --verify -vvvv
$ forge script script/upgrades/ERC20Manager.s.sol:ERC20ManagerScript --rpc-url $HOODI_RPC_URL --broadcast --verify -vvvv

$ forge script script/upgrades/MessageQueue.s.sol:MessageQueueScript --rpc-url $LOCAL_RPC_URL --broadcast -vvvv
$ forge script script/upgrades/MessageQueue.s.sol:MessageQueueScript --rpc-url $MAINNET_RPC_URL --broadcast --verify -vvvv
$ forge script script/upgrades/MessageQueue.s.sol:MessageQueueScript --rpc-url $SEPOLIA_RPC_URL --broadcast --verify -vvvv
$ forge script script/upgrades/MessageQueue.s.sol:MessageQueueScript --rpc-url $HOODI_RPC_URL --broadcast --verify -vvvv

$ forge script script/upgrades/WrappedVara.s.sol:WrappedVaraScript --rpc-url $LOCAL_RPC_URL --broadcast -vvvv
$ forge script script/upgrades/WrappedVara.s.sol:WrappedVaraScript --rpc-url $MAINNET_RPC_URL --broadcast --verify -vvvv
$ forge script script/upgrades/WrappedVara.s.sol:WrappedVaraScript --rpc-url $SEPOLIA_RPC_URL --broadcast --verify -vvvv
$ forge script script/upgrades/WrappedVara.s.sol:WrappedVaraScript --rpc-url $HOODI_RPC_URL --broadcast --verify -vvvv
```

### Preparing the ZK-to-BEEFY governance payload

After satisfying the mainnet migration gates above, use independently reviewed
addresses and cut heights to generate calldata; this does not authorize execution.

```bash
MIGRATION_DATA=$(cast calldata "migrateToBeefy(address,address,uint256,uint256,address)" "$LEGACY_VERIFIER" "$BEEFY_VERIFIER" "$LEGACY_MAX_BLOCK" "$FIRST_BEEFY_BLOCK" "$BEEFY_RECOVERY_TIMELOCK")
cargo run --package governance-tool --release -- --ethereum-endpoint "$MAINNET_RPC_URL" GovernanceAdmin UpgradeProxy MessageQueue "$NEW_IMPLEMENTATION" "$MIGRATION_DATA"
```

The queue must already be paused. Execute only via the approved existing source
governance authority and retain the original message, proof and transaction identity.
Do not retry an uncertain governance message before reconciling its original outcome.

### Coverage

We use Docker image and slightly modified sources to produce coverage report in HTML format.

```shell
$ docker build --tag gear-bridges/ethereum-contracts .
$ docker run --rm -it --volume "$(pwd)":/files gear-bridges/ethereum-contracts

$ sed -i '/if (!BinaryMerkleTree.verifyProofCalldata(merkleRoot, proof, totalLeaves, leafIndex, messageHash)) {/{N;N;d;}' src/MessageQueue.sol
$ sed -i '/emit MessageProcessed(blockNumber, messageHash, message.nonce, message.destination);/d' src/MessageQueue.sol
$ sed -i '/function test_ProcessMessageWithInvalidMerkleProof()/,/^[[:space:]]*}[[:space:]]*$/d' test/MessageQueue.t.sol
$ forge test
$ forge coverage --report lcov
$ genhtml lcov.info --branch-coverage --output-dir /files/coverage
$ exit

$ docker image rm gear-bridges/ethereum-contracts
$ docker buildx prune --all

$ sudo chown -R $(whoami):$(whoami) coverage
$ firefox coverage/index.html
```

### Cast

```shell
$ cast <subcommand>
```

### Help

```shell
$ forge --help
$ anvil --help
$ cast --help
```
