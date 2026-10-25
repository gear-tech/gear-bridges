## The **vft-vara** program

This branch's retained production compatibility route uses the existing immutable
`EscrowV1` wrapper only. Keep every VFT/VFT-VARA ActorId, CodeId, ledger, supply,
allowance and backing, and the existing Ethereum token contracts. No wrapper
migration, new token, held/v2 production path or production activation is
authorized here. If a manager is replaced, construct a new manager and retain the
old manager's evidence; an in-place code/state upgrade is not supported.

Source-level v2 fixes below describe only separately qualified v2 deployments;
they cannot patch retained immutable v1 code. A2 partial/excess recovery, A6 hold
consent and P0-R1 held-liability release remain production blockers under the
retained-v1 constraint. Require a complete admission/reservation inventory and
the actual v1 `OutstandingRedemptions == 0` before `BeginCutover` and again
paused/drained before custody transfer. `BeginCutover` alone is not that proof;
v2 blocking-zero or reservation resets cannot substitute for it.

The program workspace includes the following packages:
- `vft-vara` is the package allowing to build WASM binary for the program and IDL file for it. 
- `vft-vara-app` is the package containing business logic for the program represented by the `VftVaraService` structure.  
- `vft-vara-client` is the package containing the client for the program allowing to interact with it from another program, tests, or
  off-chain client.

## Native obligations and wrapper lifetime

The wrapper is a permanent native-backing and original-reply owner. Its derived
`VftAdmin` extends the pinned Sails 0.9.2 base service and shadows
`Exit(inheritor)` and `SetMinimumBalance`: exit always rejects, including while
paused and with zero supply; the VFT minimum must remain zero. A supply or
escrow-only exit guard cannot account for inherited native-exchange reply paths.
The base admin service is internal, not a second public exit route.
Native-wrapper replacement is unsupported. Manager replacement must retain this
same wrapper.

`NativeEscrow::OutstandingRedemptions()` is a constant-time count, initially zero.
Each new redemption increments it once. An authenticated original payout
child/recipient reply settles it as `Delivered` only on success with zero
returned value. A success or error returning at least the original amount commits
`Queued` with the actual `returned_value`: backed wrapped credit is still owed.
The callback never mints, so a capacity failure cannot consume the only reply
and roll back its evidence. An identical manager `RedeemEscrow` retries the
credit in a separate command, including while paused, and only then records
`Refunded` and decrements the obligation. Credit is exactly the original amount;
excess remains wrapper-owned native backing. A failed credit rolls back only that
retry; resolve balance capacity and retry the same operation. The recipient is the **original
Gear recipient**, never the manager or another destination, with no second
native payout. `Refunded` is not native delivery. Duplicate calls/replies cannot
mint again, decrement again or replace committed returned backing. Unheld
`Queued`, `Returned` and `Ambiguous` originals with backing at least the amount
can retry credit; partial or unknown backing cannot.
`NativeEscrow::HoldRedemption(operation_id, expected, acknowledge_irreversible_hold)`
requires the wrapper admin and pause. It retains the exact queried snapshot of
an existing Queued, Returned or Ambiguous payout with its nonzero original child.
Queued payouts and full/excess-backed payouts require explicit `true` consent:
holding irreversibly disables wrapped-credit retry. Partial ambiguous holds may
pass `false`. It does not settle, mint, forgive debt or change the original
redemption. Matching repeated holds are idempotent, including after the original
callback advances. Conflicting snapshots and terminal payouts reject; excess
backing alone does not. No release or payout API is introduced (P0-R1 remains open).

`BlockingRedemptions()` subtracts held outstanding liabilities from the financial
`OutstandingRedemptions()` count. Paused `ConfigureManager` requires zero blocking
obligations. Rotation retains original redemptions, payout children and hold
snapshots. A held operation cannot dispatch or receive wrapped credit again.
Its original callback can still record delivery or returned backing after rotation;
delivery decrements both outstanding counters once, without deleting the saved
hold. A full return after holding records backing but does not mint.

Resume the wrapper after confirming holds and before source custody transfer;
paused token transfers would otherwise block upgrade. Keep the source manager
paused for bounded receipt resolution. The existing role handoff pauses the
wrapper again when rotating its manager and burner.

The minimum is zero at construction so fractional burns never reap pooled
manager escrow into awesome-sails `unused` balances (for example, a 1.5 VARA
escrow split into 1 and 0.5 VARA claims). Configure/deployment rejects preexisting
nonzero-minimum wrappers rather than lowering the minimum or minting a dust
repair. These fixes require a newly deployed immutable program **before bridge
exposure**; they cannot patch an old deployed wrapper or reconstruct already
reaped dust. An affected existing exposed wrapper remains a production blocker;
any liability-preserving wrapper migration requires separate qualification and
authorization outside this retained-v1 route. Manager cutover cannot replace it.

Manager ReconcileReceipt first validates the wrapper's existing Redemption.
An unheld `Queued`, `Returned` or `Ambiguous` record with backing at least the
amount permits the identical `RedeemEscrow` retry for wrapped credit; it never
creates a missing redemption, burns again or sends a replacement native payout.
Live `Delivered`/`Refunded` evidence takes precedence over a saved historical hold;
both saved and live records must be non-terminal for a new manager Held resolution.
An already-created manager Held row is not rewritten after delivery. Cutover
evidence can separately authenticate a saved Queued/zero-return hold and its
exact original Delivered/zero-return payout at one finalized pin as a historical
exception, never as a new active unpaid hold or normal financial completion.
Query and retry gas, reply deposit and timeout use existing configuration.
Missing or ambiguous economic evidence remains unresolved. New native amounts
are bounded to `2^80−1` base units, matching awesome-sails balances.
Separately signed read-only continuation replacement requires durable exact-byte archival and
authenticated terminal no-effect evidence, never merely a missing reply.

Manual pause and emergency stop still fence new bridge exposure. Authorized
owner/admin recovery may return only a definitely refundable original source
request; pending/ambiguous builtin or token outcomes cannot authorize a refund.
Historical completion requalification requires the original proof/reply and
full manager/wrapper state at the saved canonical finalized settlement pin.
Keep that archive available: evicted live receipt rows, a Processed-only flag or
successor state cannot substitute for original economic evidence.
