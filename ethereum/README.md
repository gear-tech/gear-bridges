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
$ source .env

$ forge script script/Deployment.s.sol:DeploymentScript --rpc-url $MAINNET_RPC_URL --broadcast --verify -vvvv
$ forge script script/Deployment.s.sol:DeploymentScript --rpc-url $SEPOLIA_RPC_URL --broadcast --verify -vvvv
$ forge script script/Deployment.s.sol:DeploymentScript --rpc-url $HOLESKY_RPC_URL --broadcast --verify -vvvv
$ forge script script/Deployment.s.sol:DeploymentScript --rpc-url $HOODI_RPC_URL --broadcast --verify -vvvv
```

### Upgrade

> [!WARNING]
> Before you run upgrade scripts, edit `reinitialize` method depending on how you want to perform upgrade (only for `WrappedVara`, `ERC20Manager`, `MessageQueue`)!
> If this is not first update, then do not forget to bump version in `reinitializer(/*version*/)` modifier.

```shell
$ source .env

$ forge script script/upgrades/WrappedVara.s.sol:WrappedVaraScript --rpc-url $MAINNET_RPC_URL --broadcast --verify -vvvv
$ forge script script/upgrades/WrappedVara.s.sol:WrappedVaraScript --rpc-url $SEPOLIA_RPC_URL --broadcast --verify -vvvv
$ forge script script/upgrades/WrappedVara.s.sol:WrappedVaraScript --rpc-url $HOLESKY_RPC_URL --broadcast --verify -vvvv
$ forge script script/upgrades/WrappedVara.s.sol:WrappedVaraScript --rpc-url $HOODI_RPC_URL --broadcast --verify -vvvv

$ forge script script/upgrades/ERC20Manager.s.sol:ERC20ManagerScript --rpc-url $MAINNET_RPC_URL --broadcast --verify -vvvv
$ forge script script/upgrades/ERC20Manager.s.sol:ERC20ManagerScript --rpc-url $SEPOLIA_RPC_URL --broadcast --verify -vvvv
$ forge script script/upgrades/ERC20Manager.s.sol:ERC20ManagerScript --rpc-url $HOLESKY_RPC_URL --broadcast --verify -vvvv
$ forge script script/upgrades/ERC20Manager.s.sol:ERC20ManagerScript --rpc-url $HOODI_RPC_URL --broadcast --verify -vvvv

$ forge script script/upgrades/MessageQueue.s.sol:MessageQueueScript --rpc-url $MAINNET_RPC_URL --broadcast --verify -vvvv
$ forge script script/upgrades/MessageQueue.s.sol:MessageQueueScript --rpc-url $SEPOLIA_RPC_URL --broadcast --verify -vvvv
$ forge script script/upgrades/MessageQueue.s.sol:MessageQueueScript --rpc-url $HOLESKY_RPC_URL --broadcast --verify -vvvv
$ forge script script/upgrades/MessageQueue.s.sol:MessageQueueScript --rpc-url $HOODI_RPC_URL --broadcast --verify -vvvv

$ forge script script/upgrades/VerifierMock.s.sol:VerifierMockScript --rpc-url $MAINNET_RPC_URL --broadcast --verify -vvvv
$ forge script script/upgrades/VerifierMock.s.sol:VerifierMockScript --rpc-url $SEPOLIA_RPC_URL --broadcast --verify -vvvv
$ forge script script/upgrades/VerifierMock.s.sol:VerifierMockScript --rpc-url $HOLESKY_RPC_URL --broadcast --verify -vvvv
$ forge script script/upgrades/VerifierMock.s.sol:VerifierMockScript --rpc-url $HOODI_RPC_URL --broadcast --verify -vvvv

$ forge script script/upgrades/VerifierMainnet.s.sol:VerifierMainnetScript --rpc-url $MAINNET_RPC_URL --broadcast --verify -vvvv
$ forge script script/upgrades/VerifierMainnet.s.sol:VerifierMainnetScript --rpc-url $SEPOLIA_RPC_URL --broadcast --verify -vvvv
$ forge script script/upgrades/VerifierMainnet.s.sol:VerifierMainnetScript --rpc-url $HOLESKY_RPC_URL --broadcast --verify -vvvv
$ forge script script/upgrades/VerifierMainnet.s.sol:VerifierMainnetScript --rpc-url $HOODI_RPC_URL --broadcast --verify -vvvv

$ forge script script/upgrades/VerifierTestnet.s.sol:VerifierTestnetScript --rpc-url $MAINNET_RPC_URL --broadcast --verify -vvvv
$ forge script script/upgrades/VerifierTestnet.s.sol:VerifierTestnetScript --rpc-url $SEPOLIA_RPC_URL --broadcast --verify -vvvv
$ forge script script/upgrades/VerifierTestnet.s.sol:VerifierTestnetScript --rpc-url $HOLESKY_RPC_URL --broadcast --verify -vvvv
$ forge script script/upgrades/VerifierTestnet.s.sol:VerifierTestnetScript --rpc-url $HOODI_RPC_URL --broadcast --verify -vvvv
```

### Example of changing `Verifier*.sol`

1. deploy new `Verifier*` (see the previous chapter for details)

2. add the method to the `MessageQueue`:

   ```solidity
   /**
    * @custom:oz-upgrades-validate-as-initializer
    */
   function reinitialize() public onlyRole(DEFAULT_ADMIN_ROLE) reinitializer(/*version*/ 2) {
       _verifier = IVerifier(/*address of `Verifier*`*/ 0x0000000000000000000000000000000000000000);
   }
   ```

3. deploy the new `MessageQueue` and write the address to `NEW_IMPLEMENTATION`

4. generate an update message with the help of `governance-tool` (see `tools/governance/README.md`):

   ```bash
   NEW_IMPLEMENTATION="0x0000000000000000000000000000000000000000" # must exist on https://etherscan.io
   cargo run --package governance-tool --release -- --rpc-url $MAINNET_RPC_URL GovernanceAdmin UpgradeProxy MessageQueue $NEW_IMPLEMENTATION $(cast calldata "function reinitialize()")
   ```

5. send the extrinsic `gearEthBridge::sendEthMessage` in behalf of `governance admin`

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
