## The **vft-vara** program

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
returned value. An error returning exactly the full original amount commits
`Queued` with that full `returned_value`: backed wrapped credit is still owed.
The callback never mints, so a capacity failure cannot consume the only reply
and roll back its evidence. An identical manager `RedeemEscrow` retries the
credit in a separate command, including while paused, and only then records
`Refunded` and decrements the obligation. A failed credit rolls back only that
retry; allocate additional balance storage (or resolve the recipient balance
limit) and retry the same operation. The recipient is always the **original
Gear recipient**, never the manager or another destination, with no second
native payout. `Refunded` is not native delivery. Duplicate calls/replies cannot
mint again, decrement again or replace committed returned backing; legacy
`Returned`, unreturned `Queued` and `Ambiguous` remain held obligations.
`NativeEscrow::HoldRedemption(operation_id, expected)` requires the wrapper admin
and pause. It retains the exact queried snapshot of an existing Queued, Returned
or Ambiguous payout with its nonzero original child. It does not settle, mint,
forgive debt, or change the original redemption. Matching repeated holds are
idempotent, including after the original callback advances the live record.
Conflicting snapshots, terminal payouts and returned value above the original
amount reject.

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
reaped dust. An existing exposed wrapper needs an independently qualified,
liability-preserving migration; manager cutover alone cannot replace it.

Manager ReconcileReceipt first queries the wrapper's existing Redemption.
Only an original `Queued` record with exactly full returned backing allows it
to repeat the identical `RedeemEscrow` for wrapped credit; it never creates a
missing redemption, burns again or sends a replacement native payout. Query
and retry gas, reply deposit and timeout use the manager's existing
configuration. A send, timeout, decode or credit failure leaves NativeQueued
and original operation/child identities unchanged. A later bounded recovery
may settle the original delivered or wrapped-refunded payout; missing or
ambiguous economic evidence remains held.
Separately signed read-only continuation replacement requires durable exact-byte archival and
authenticated terminal no-effect evidence, never merely a missing reply.

Manual pause and emergency stop still fence new bridge exposure. Authorized
owner/admin recovery may return only a definitely refundable original source
request; pending/ambiguous builtin or token outcomes cannot authorize a refund.
Historical completion requalification requires the original proof/reply and
full manager/wrapper state at the saved canonical finalized settlement pin.
Keep that archive available: evicted live receipt rows, a Processed-only flag or
successor state cannot substitute for original economic evidence.
