## Vara Bridge

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

Bytes[0..=0x1F] VaraAddress
Bytes[0x20..=0x33] EthAddress
Nonce[0x34..=0x53] Nonce
Data[0x54..]

./interfaces/IMessageQueue.sol

```
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
    return (bytes(""));
}
```

## Deploy

### Anvil

```bash
forge script DeployScript --fork-url http://localhost:8545 --broadcast --private-key 0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80
```

```
Prover: 0xe7f1725E7734CE288F8367e1Bb143E90bb3F0512
Relayer: 0x9fE46736679d2D9a65F0992F2272dE9f3c7fa6e0
Treasury: 0xCf7Ed3AccA5a467e9e704C703E8D87F634fB0Fc9
MessageQueue: 0xCf7Ed3AccA5a467e9e704C703E8D87F634fB0Fc9
Relayer Proxy: 0x5FC8d32690cc91D4c39d9d3abcBD16989F875707
Treasury Proxy: 0xa513E6E4b8f2a923D98304ec87F64353C4D5C853
MessageQueue Proxy: 0x0165878A594ca255338adfa4d48449f69242Eb8F
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
$ forge script script/Counter.s.sol:CounterScript --rpc-url <your_rpc_url> --private-key <your_private_key>
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
