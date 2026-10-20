## The **vft-manager** program

The program workspace includes the following packages:
- `vft-manager` is the package allowing to build WASM binary for the program and IDL file for it.  
  The package also includes integration tests for the program in the `tests` sub-folder
- `vft-manager-app` is the package containing business logic for the program represented by the `VftManagerService` structure.  
- `vft-manager-client` is the package containing the client for the program allowing to interact with it from another program, tests, or
  off-chain client.

The manager WASM build uses release-equivalent LTO, codegen, debug-assertion and
overflow-check settings even when the host CLI is built in development mode.
These settings apply only to the manager's child Cargo build, preserving other
program artifacts. Qualify the complete artifact against the target runtime:
raw WASM size alone does not establish that its instrumented code fits.

The `mocks` feature enables gas benchmarks and can be forwarded into the embedded
WASM by a full workspace/all-targets build. It is not a security boundary. Only
the current manager admin may call `FillTransactions`, `CalculateGasForReply`,
and `CalculateGasForTokenMapSwap`, including as the origin of gas estimation.
`GasCalculation` initializes a new caller-owned benchmark program; it cannot
change an initialized manager.

`CalculateGasForReply` reserves an unused receipt key and installs one synthetic,
non-native deposit before calling the production per-log mint/unlock path. Its
original child and deposit outcome remain visible in `ReceiptDeposits`; successful
replies settle the receipt, while ambiguous replies retain its reservation. It
never overwrites completed or pending receipts. Use a fresh key for each sample.
Mint success replies are empty; TransferFrom success replies contain the route
and `true`. Native redemption is covered by the receipt settlement tests, not this
mint/unlock mailbox benchmark.

`Transactions` exports only completed receipt replay keys, in the existing paginated
order. Reserved keys are not migration records; use `ReceiptStatus` and
`ReceiptDeposits` to inspect unresolved settlement. An explicit VFT error reply
is retryable for the same receipt. A successful transport reply containing
`false` is quarantined, not redispatched.

Native redemption requires configuring the manager's zero-VFT-minimum native
wrapper and the wrapper's escrow manager while both are paused. Configuration
rejects a nonzero minimum; it does not silently repair previously reaped escrow.
A queued mailbox payout is not settled until its original reply is authenticated.
An original recipient error returning the exact full payout first records durable
backing in the wrapper. Reconciliation retries wrapped credit for that original
recipient; capacity failure retains the backing and outstanding obligation for
another retry, without another native payout. Only successful credit records
`ReceiptDepositOutcome::Refunded`, not native `Settled`/`Delivered`, and
releases the completed receipt reservation. Refunds emit the distinct
`ReceiptDepositRefunded` event, never the native-delivery settlement or
`BridgingAccepted` event. It binds the original receipt/log, operation, wrapper,
recipient, amount, manager child, payout child and full returned backing. The
finalized indexer/history outcome is `Refunded` (wrapped), not native delivered.
Duplicate callbacks, reconciliation and receipt
replay cannot mint again or enqueue another native payout. Partial, ambiguous
and legacy `Returned` evidence stays held. Fully resolved wrapped refunds no
longer veto manager/wrapper-manager cutover.

### Retained-program cutover

Manager replacement keeps the original manager queryable forever and keeps the
same native wrapper. Replay-key export/import and manager exit are not a
migration protocol: `InsertTransactions` is removed and `Upgrade` never exits,
clears mappings, transfers native operating reserves, or deletes original
receipt/source/native/transfer evidence. These guards require new immutable
programs; they cannot retrofit an already-deployed old manager.

The admin lifecycle is:

1. `BeginCutover(destination, last_receipt_slot)` freezes economic configuration,
   token mappings and native policy, and enters `Draining`. The source accepts
   only receipts at or below the authenticated Beacon-slot boundary. It rejects
   boundaries below its permanent maximum admitted receipt slot, including
   reserved receipts. Identical retries preserve the intent; changed intents,
   zero/self destinations and boundary overflow reject. Outgoing
   `RequestBridging` is fenced even after `Unpause`. Pre-boundary settlement and
   definite authorized refunds remain available; manual/emergency pause still
   gates fresh receipts.
2. Pause the drained source. An authorized repeated `Pause` is idempotent only
   in `Draining`, preserving a resumable original pause call even if already paused.
   On a paused, unused replacement with the identical
   economic configuration, call `PrepareCutover(source, last_receipt_slot)`.
   The replacement authenticates the source status, freezes configuration, sets
   the immutable floor to boundary + 1, and enters `AwaitingSource`.
   Preparation rechecks pause, permanent economic-activity state, quiescence and
   configuration revision after the source query. `Unpause` cannot activate it.
3. `Upgrade(destination)` checks the prepared destination, the paused source's
   local obligations, and the same wrapper's manager and zero outstanding
   redemptions. It enters irreversible `Transferring` and transfers the frozen,
   sorted token inventory. Each enqueue and original child record is atomic.
   Concurrent continuation calls cannot claim a token twice. Continue this same
   cutover until every token is completed and final token balances are zero;
   only then does the source enter irreversible `Retired`.
4. Handoff existing token roles and the wrapper manager/burner through their
   authorized lifecycle. Wrapper backing and original payout records stay at
   that wrapper. `FinalizeCutover` authenticates matching retired source status,
   completed custody, destination wrapper manager/burner and zero outstanding
   redemptions, then enters `Ready` while remaining paused. Only `Unpause`
   changes `Ready` to `Active`. A later cutover retains the predecessor and
   the existing receipt floor.

All receipt dispatch and reconciliation paths enforce the deployment domain
before decoding or economic work. Below-floor/above-boundary receipts return
`ReceiptOutsideDeployment`, retired dispatch returns `Retired`, and premature
activation/outgoing exposure returns `CutoverNotReady`. An old evicted replay
key cannot be reminted at the successor because its entire receipt domain is
excluded, not copied into another bounded history.

`has_economic_activity` is permanent, not inferred from retained history. All
named economic mutation routes, including mock benchmarks/history filling, are
frozen during cutover and retirement. Admin/pause-admin/observer rotation does
not alter the frozen economic identity. Awaiting VFT-map updates recheck phase
and revision before committing so a concurrent cutover cannot be overwritten.

`CutoverStatus` exposes phase, predecessor, successor, minimum receipt slot,
optional last receipt slot, optional configuration fingerprint, token transfer
count, completed token count and unresolved original-child count, in that SCALE
order. Zero-balance tokens count as completed without creating fake transfer
attempts. `CutoverTransfers(start, count)` retains every original attempt with
`token, amount, child, outcome`. Exact routed `true` completes; explicit error
or exact `false` rejects and permits a later appended retry. Timeout, missing
reply, malformed success or trailing bytes remain in-flight/ambiguous and never
permit a competing child. Authenticated late replies resolve the same child;
terminal attempts cannot be rewritten. Queries and transfers use the frozen
configured token-operation gas, reply deposit and block deadline. A timeout
ends only the wait, not original custody ownership. Lifecycle commands return
unit; operators must read status to distinguish pending custody from retirement.
No new lifecycle events are invented: original settlement/request events remain
unchanged, and lifecycle progress is read through these retained queries.

The fingerprint is Keccak256 of SCALE
`(*b"vft-manager/cutover-config/v1", builtin, proxy, erc20_manager,
native_wrapper, config, sorted_mappings)`. Mappings are sorted by raw ActorId
bytes and reject duplicate Gear/Ethereum identities. Config declaration order
remains token-op gas, reply-deposit gas, builtin gas, map-swap gas, reply timeout,
bridge fee and incoming fee. Admin/observers, pause, domains and progress are not
part of this economic fingerprint. It is frozen once rather than recomputed
for each custody transfer.

Quiescence checks receipt reservations/nonterminal receipt children, the
incremental pending-original-builtin count, source deposit/refund children and
only terminal tracker states: failed deposit, queued source response, or
completed refund. A reservation moves into completed history only when its
nonempty per-log progress is entirely processed. These checks prove admitted
Gear obligations, **not coverage of unseen Ethereum deposits**.

The operator must separately authenticate the boundary, complete all canonical
receipt-root coverage (including explicitly sponsored authentic unpaid
pre-boundary deposits), drain original signed attempts/payouts/refunds, and
retain original finalized evidence. Use the resumable prepare/finalize-cutover
protocol and immutable seal described in the runbook, not copied keys or a
fresh deployment as proof of migration. Old journals are sealed and verified
read-only; successor journals bind the exact seal and begin discovery at the
boundary execution block + 1. No rollback to an active source is supported
after custody transfer starts.


Local regression command (actual gtest WASM, no node required):
`cargo test --release -p tests vft_manager::gtest:: -- --test-threads=1`.
