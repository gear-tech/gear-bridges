## Solidity smart contracts

./interfaces/IMessageQueue.sol

```solidity
struct VaraMessage {
    bytes32 vara_address;
    address eth_address;
    uint256 nonce;
    bytes data;
}

function process_message(VaraMessage calldata message) external;
```

- Bytes[0..=0x1F] VaraAddress
- Bytes[0x20..=0x33] EthAddress
- Bytes[0x34..=0x53] Nonce
- Bytes[0x54..]

./interfaces/ITreasury.sol

```solidity
struct WithdrawMessage {
    address receiver;
    address token;
    uint128 amount;
}
```

- Bytes[0..=0x13] Receiver
- Bytes[0x14..=0x27] Token
- Bytes[0x28..=0x37] Amount

./interfaces/IMessageQueue.sol

```solidity
library Hasher {
    function hash(VaraMessage calldata message) external pure returns (bytes32) {
        bytes memory data = abi.encodePacked(message.vara_address, message.eth_address, message.nonce, message.data);
        return keccak256(data);
    }
}
```

./Treasury.sol

```solidity
fallback(bytes calldata data) onlyRole(Constants.MESSAGE_QUEUE_ROLE) external returns (bytes memory){
    (address token, address to, uint256 amount) = abi.decode(data, (address, address, uint256));
    withdraw(token, to, amount);
    return new bytes(0);
}
```

## Setup

```bash
git clone --recurse-submodules https://github.com/gear-tech/gear-bridges.git
cd gear-bridges/ethereum
cp .env.example .env # don't forget to modify `.env`
```

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
$ forge script script/DeployMockERC20.s.sol:DeployMockERC20Script --rpc-url $HOLESKY_RPC_URL --broadcast --verify -vvvv

$ source .env
$ forge script script/DeployWrappedEther.s.sol:DeployWrappedEtherScript --rpc-url $HOLESKY_RPC_URL --broadcast --verify -vvvv

$ source .env
$ forge script script/DeployCore.s.sol:DeployCoreScript --rpc-url $HOLESKY_RPC_URL --broadcast --verify -vvvv

$ source .env
$ forge script script/DeployTokenBridge.s.sol:DeployTokenBridgeScript --rpc-url $HOLESKY_RPC_URL --broadcast --verify -vvvv

$ source .env
$ forge script script/DeployWrappedVara.s.sol:DeployWrappedVaraScript --rpc-url $HOLESKY_RPC_URL --broadcast --verify -vvvv
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
