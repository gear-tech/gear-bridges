# Ethereum smart contracts ABI and gear programs idl files

Contents of this folder are guaranteed to be up to date as there's a workflow in CI that checks that they match with ones generated when the project is built.

> [!WARNING]
> Update CI workflow when adding new files to this folder

Regenerate the Ethereum snapshots from the repository root with the CI-pinned
Foundry v1.8.5 and the compiler/settings in `ethereum/foundry.toml` (Solc 0.8.37):

```sh
forge build --force --no-cache --root ethereum
for contract in IBridgingPayment IERC20Manager IMessageQueue WrappedEther ERC20Manager MessageQueue BridgingPayment BeefyClient VaraQueueRootVerifier; do
  cp "ethereum/out/$contract.sol/$contract.json" "api/ethereum/$contract.json"
done
```

Commit the complete generated artifacts, not just their ABI fields. Solidity AST
IDs and source coordinates depend on the full compilation input, so changes to
tests or scripts can require refreshing otherwise unchanged contract snapshots.
CI rebuilds the same complete input and compares every snapshot byte-for-byte.