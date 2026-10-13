## The **vft-vara** program

The program workspace includes the following packages:
- `vft-vara` is the package allowing to build WASM binary for the program and IDL file for it. 
- `vft-vara-app` is the package containing business logic for the program represented by the `VftVaraService` structure.  
- `vft-vara-client` is the package containing the client for the program allowing to interact with it from another program, tests, or
  off-chain client.

## Native obligations and wrapper lifetime

The wrapper is a permanent native-backing and original-reply owner. Its derived
`VftAdmin` extends the pinned Sails 0.9.2 base service and shadows only
`Exit(inheritor)`: it checks the admin and always rejects, including while paused,
with zero supply or after all escrow redemptions have been delivered. A supply or
escrow-only exit guard cannot account for inherited native-exchange reply paths.
Ordinary VFT, burn and native-exchange administration remain unchanged; the base
admin service is internal, not a second public exit route. Native-wrapper
replacement is unsupported. Manager replacement must retain this same wrapper.

`NativeEscrow::OutstandingRedemptions()` is a constant-time count, initially zero.
Each new redemption increments it once; only an authenticated reply from the
original payout child/recipient changing Queued to Delivered decrements it.
Duplicate calls/replies do not change the count. Returned or Ambiguous payouts
remain obligations, including native value returned to the wrapper. While
paused, ConfigureManager requires a zero count. Rotation never rewrites the
original Redemption.from, amount, recipient, child or reply records.

Manager ReconcileReceipt only queries the wrapper's existing Redemption; it
does not burn, mint, unlock or send a replacement native payout. Query gas,
reply deposit and timeout use the manager's existing configuration. A send,
timeout or decode failure leaves NativeQueued and original operation/child
identities unchanged. A later bounded query may settle the original delivered
payout; missing or ambiguous economic evidence remains held. Separately signed
read-only continuation replacement requires durable exact-byte archival and
authenticated terminal no-effect evidence, never merely a missing reply.

Manual pause and emergency stop still fence new bridge exposure. Authorized
owner/admin recovery may return only a definitely refundable original source
request; pending/ambiguous builtin or token outcomes cannot authorize a refund.
Historical completion requalification requires the original proof/reply and
full manager/wrapper state at the saved canonical finalized settlement pin.
Keep that archive available: evicted live receipt rows, a Processed-only flag or
successor state cannot substitute for original economic evidence.
