## The **wrapped-vara** program

**wrapped-vara** allows you to mint an amount from `value` to an account, and burn and return `value`.

The program workspace includes the following packages:
- `wrapped-vara` is the package allowing to build WASM binary for the program and IDL file for it.  
  The package also includes integration tests for the program in the `tests` sub-folder
- `wrapped-vara-app` is the package containing business logic for the program represented by the `TokenizerService` structure.
- `wrapped-vara-client` is the package containing the client for the program allowing to interact with it from another program, tests, or
  off-chain client.

