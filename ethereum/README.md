## Foundry

We use Foundry - https://book.getfoundry.sh/

## Usage

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

### Example of changing Verifier.sol

1. deploy new `Verifier` (see the previous chapter for details)
1. add the method to the `MessageQueue`:
```
    /**
     * @custom:oz-upgrades-validate-as-initializer
     */
    function reinitialize() public onlyRole(DEFAULT_ADMIN_ROLE) reinitializer(2) {
        _verifier = IVerifier(0x0001...09);
    }
```
1. deploy the new `MessageQueue`
1. generate an update message with the help of `governance-tool`:
```
./target/release/governance-tool --rpc-url $RPC_URL GovernanceAdmin UpgradeProxy MessageQueue <MQ_IMPL> $(cast calldata "function reinitialize()")
```
1. send the extrinsic `gearEthBridge::sendEthMessage` in behalf of `governance admin`


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
