# MessageHandler Contract

This directory contains the MessageHandler smart contract and deployment scripts for the Gear Bridges project.

## Overview

The `MessageHandler` contract implements the `IMessageHandler` interface and provides functionality to handle messages from the bridge system.

### Contract Features
- Implements `IMessageHandler` interface
- Emits `MessageHandled` events for tracking
- Simple message handling with source tracking

## Prerequisites

Make sure you have the following installed:
- [Foundry](https://book.getfoundry.sh/getting-started/installation)
- Node.js and npm/yarn (for the broader project)

## Setup

1. Install Foundry dependencies:
```shell
forge install
```

2. Build the contracts:
```shell
forge build
```

3. Run tests:
```shell
forge test
```

## Deployment

### Quick Deployment

The easiest way to deploy is using the provided deployment script:

```shell
# Make the script executable
chmod +x deploy.sh

# Deploy to local anvil (starts automatically)
./deploy.sh

# Deploy to Sepolia testnet
export PRIVATE_KEY=your_private_key_here
./deploy.sh sepolia PRIVATE_KEY

# Deploy to Holesky testnet  
export PRIVATE_KEY=your_private_key_here
./deploy.sh holesky PRIVATE_KEY
```

### Manual Deployment

You can also deploy manually using Forge:

```shell
# Deploy to local anvil
forge script script/DeployMessageHandler.s.sol:DeployMessageHandler \
    --fork-url http://localhost:8545 \
    --broadcast \
    --private-key 0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80

# Deploy to testnet
forge script script/DeployMessageHandler.s.sol:DeployMessageHandler \
    --rpc-url sepolia \
    --broadcast \
    --private-key $PRIVATE_KEY \
    --verify
```

### Deployment Output

After successful deployment, you'll find:
- `deployed_address.txt` - Contains the contract address
- `deployed_contract.json` - Contains deployment metadata including address, network, and timestamp

## Environment Variables

For testnet deployments, make sure to set:

```shell
export PRIVATE_KEY=your_private_key_here
export SEPOLIA_RPC_URL=your_sepolia_rpc_url
export HOLESKY_RPC_URL=your_holesky_rpc_url
export ETHERSCAN_API_KEY=your_etherscan_api_key
```

## Contract Verification

The deployment script automatically verifies contracts on testnets. For manual verification:

```shell
forge verify-contract \
    --chain-id 11155111 \
    --num-of-optimizations 200 \
    --watch \
    --constructor-args $(cast abi-encode "constructor()") \
    --etherscan-api-key $ETHERSCAN_API_KEY \
    --compiler-version v0.8.33+commit.64118f21 \
    CONTRACT_ADDRESS \
    src/MessageHandler.sol:MessageHandler
```

## Foundry Commands

### Build
```shell
forge build
```

### Test
```shell
forge test
```

### Format
```shell
forge fmt
```

### Gas Snapshots
```shell
forge snapshot
```

### Local Development (Anvil)
```shell
anvil
```

### Cast Utilities
```shell
cast <subcommand>
```

### Help
```shell
forge --help
anvil --help
cast --help
```

## Project Structure

```
contracts/
├── script/
│   └── DeployMessageHandler.s.sol    # Deployment script
├── src/
│   └── MessageHandler.sol             # Main contract
├── deploy.sh                          # Deployment helper script
├── deployed_address.txt              # Contract address (generated)
├── deployed_contract.json            # Deployment metadata (generated)
├── foundry.toml                      # Foundry configuration
└── README.md                         # This file
```

## Documentation

- [Foundry Book](https://book.getfoundry.sh/)
- [Gear Protocol Documentation](https://wiki.gear-tech.io/)