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
