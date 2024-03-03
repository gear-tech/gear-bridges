## Vara Bridge 

```solidity
struct VaraMessage {
    uint256 block_number;
    ContentMessage content;
    bytes proof;
}


struct ContentMessage  {
    address eth_address;
    bytes32 vara_address;
    uint256 nonce;
    bytes data;
}

function process_message(VaraMessage calldata message ) external;
```




```solidity
library Hasher {
    function hash(ContentMessage calldata message) external pure returns(bytes32) {
        bytes memory data = abi.encodePacked(message.eth_address, message.vara_address, message.nonce, message.data);
        return keccak256(data);
    }
}
```
data = [0..0x13]eth_address[0x14..0x33]vara_address[0x34..0x53]nonce[0x54..]data


```solidity
    fallback(bytes calldata data) onlyRole(Constants.MESSAGE_QUEUE_ROLE) external returns (bytes memory){
        (address token, address to, uint256 amount ) = abi.decode(data, (address, address, uint256));
        withdraw(token, to, amount);
        return( bytes("") );
    }
```



## Foundry

**Foundry is a blazing fast, portable and modular toolkit for Ethereum application development written in Rust.**

Foundry consists of:

-   **Forge**: Ethereum testing framework (like Truffle, Hardhat and DappTools).
-   **Cast**: Swiss army knife for interacting with EVM smart contracts, sending transactions and getting chain data.
-   **Anvil**: Local Ethereum node, akin to Ganache, Hardhat Network.
-   **Chisel**: Fast, utilitarian, and verbose solidity REPL.

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
