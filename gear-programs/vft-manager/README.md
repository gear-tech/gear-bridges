## The **vft-manager** program

The program workspace includes the following packages:
- `vft-manager` is the package allowing to build WASM binary for the program and IDL file for it.  
  The package also includes integration tests for the program in the `tests` sub-folder
- `vft-manager-app` is the package containing business logic for the program represented by the `VftManagerService` structure.  
- `vft-manager-client` is the package containing the client for the program allowing to interact with it from another program, tests, or
  off-chain client.

The `mocks` feature enables gas benchmarks and can be forwarded into the embedded
WASM by a full workspace/all-targets build. It is not a security boundary. Only
the current manager admin may call `FillTransactions`, `CalculateGasForReply`,
and `CalculateGasForTokenMapSwap`, including as the origin of gas estimation.
`GasCalculation` initializes a new caller-owned benchmark program; it cannot
change an initialized manager.

`Transactions` includes both completed and reserved receipts; use `ReceiptStatus`
and `ReceiptDeposits` to distinguish settlement from a retryable rejection or an
ambiguous reply. An explicit VFT error reply is retryable for the same receipt. A
successful transport reply containing `false` is quarantined, not redispatched.

Native redemption requires configuring the manager's native wrapper and the
wrapper's escrow manager while both are paused. A queued mailbox payout is not
settled until the original value is claimed and its reply is reconciled. Rejected
payouts retain their original child and returned native reserve: reconciliation
and receipt replay must neither re-mint tokens nor enqueue a replacement payout.
