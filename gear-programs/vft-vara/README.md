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
child/recipient reply settles it only as `Delivered` (success, zero returned value)
or `Refunded` (error, exactly the full original amount returned). In the latter
case the wrapper atomically mints exactly backed wrapped VARA to the **original
Gear recipient**, never to the manager or another destination. It does not send
a second native payout. `Refunded` is not native delivery. Duplicate calls/replies
cannot mint again or decrement again; retained legacy `Returned`, `Queued` and
`Ambiguous` remain obligations. Paused `ConfigureManager` requires zero
outstanding obligations and a zero VFT minimum. Rotation never rewrites the
original operation, from, amount, recipient, child or reply records.

The minimum is zero at construction so fractional burns never reap pooled
manager escrow into awesome-sails `unused` balances (for example, a 1.5 VARA
escrow split into 1 and 0.5 VARA claims). Configure/deployment rejects preexisting
nonzero-minimum wrappers rather than lowering the minimum or minting a dust
repair. These fixes require a newly deployed immutable program **before bridge
exposure**; they cannot patch an old deployed wrapper or reconstruct already
reaped dust. An existing exposed wrapper needs an independently qualified,
liability-preserving migration; manager cutover alone cannot replace it.

Manager ReconcileReceipt only queries the wrapper's existing Redemption; it
does not burn, mint, unlock or send a replacement native payout. Query gas,
reply deposit and timeout use the manager's existing configuration. A send,
timeout or decode failure leaves NativeQueued and original operation/child
identities unchanged. A later bounded query may settle the original delivered
or wrapped-refunded payout; missing or ambiguous economic evidence remains held.
Separately signed read-only continuation replacement requires durable exact-byte archival and
authenticated terminal no-effect evidence, never merely a missing reply.

Manual pause and emergency stop still fence new bridge exposure. Authorized
owner/admin recovery may return only a definitely refundable original source
request; pending/ambiguous builtin or token outcomes cannot authorize a refund.
Historical completion requalification requires the original proof/reply and
full manager/wrapper state at the saved canonical finalized settlement pin.
Keep that archive available: evicted live receipt rows, a Processed-only flag or
successor state cannot substitute for original economic evidence.
