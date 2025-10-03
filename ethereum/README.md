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

$ forge script script/upgrades/Verifier.s.sol:VerifierScript --rpc-url $MAINNET_RPC_URL --broadcast --verify -vvvv
$ forge script script/upgrades/Verifier.s.sol:VerifierScript --rpc-url $SEPOLIA_RPC_URL --broadcast --verify -vvvv
$ forge script script/upgrades/Verifier.s.sol:VerifierScript --rpc-url $HOLESKY_RPC_URL --broadcast --verify -vvvv
$ forge script script/upgrades/Verifier.s.sol:VerifierScript --rpc-url $HOODI_RPC_URL --broadcast --verify -vvvv
```

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
