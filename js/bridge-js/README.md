# @gear-tech/bridge-js

A TypeScript library for relaying transactions between Ethereum and Vara networks.

## Overview

The `gear-bridge-js` library provides a seamless way to create cross-chain bridges between Ethereum and Vara networks. It enables developers to relay transactions between Ethereum and Vara networks by generating cryptographic proofs of transaction inclusion and submitting them to the network.

## Installation

```bash
npm install gear-bridge-js
```

## Prerequisites

Before using this library, ensure you have:

1. Access to an Ethereum beacon chain RPC endpoint
2. A Viem public client configured for Ethereum
3. A Gear API instance connected to the Vara network
4. Deployed program IDs for:
   - Checkpoint client program
   - Historical proxy program

## Quick Start

```typescript
import { relayEthToVaraTransaction } from 'gear-bridge-js';
import { createPublicClient, http } from 'viem';
import { mainnet } from 'viem/chains';
import { GearApi } from '@gear-js/api';

// Configure Ethereum client
const ethereumClient = createPublicClient({
  chain: mainnet,
  transport: http('https://eth-mainnet.alchemyapi.io/v2/your-api-key'),
});

// Configure Vara API
const gearApi = await GearApi.create({
  providerAddress: 'wss://rpc.vara.network',
});

// Relay transaction
const result = await relayEthToVaraTransaction(
  '0x1234...', // Ethereum transaction hash
  'https://beacon-node.example.com', // Beacon chain RPC URL
  ethereumClient,
  gearApi,
  '0xabcd...', // Checkpoint client program ID
  '0xefgh...', // Historical proxy program ID
  '0xijkl...', // Target client program ID
  'MyService', // Service name on target client
  'MyMethod', // Method name on target service
  signerAccount, // Signer account
);

console.log('Transaction relayed:', result.txHash);
```

## API Reference

### `relayEthToVaraTransaction`

Relays an Ethereum transaction to the Vara network by creating a proof and submitting it through the historical proxy program.

#### Parameters

| Parameter | Type | Description |
|-----------|------|-------------|
| `transactionHash` | `0x${string}` | Transaction hash of the Ethereum transaction to relay |
| `beaconRpcUrl` | `string` | RPC URL for the Ethereum beacon chain client |
| `ethereumPublicClient` | `PublicClient` | Viem public client for Ethereum network interactions |
| `gearApi` | `GearApi` | Gear API instance for Vara network operations |
| `checkpointClientId` | `0x${string}` | ID of the checkpoint client program on Vara |
| `historicalProxyId` | `0x${string}` | ID of the historical proxy program on Vara |
| `clientId` | `0x${string}` | ID of the target client program on Vara |
| `clientServiceName` | `string` | Name of the service to call on the target client |
| `clientMethodName` | `string` | Name of the method to call on the target service |
| `signer` | `string \| KeyringPair` | Account signer for transaction signing |
| `signerOptions?` | `Partial<SignerOptions>` | Optional signing configuration parameters |
| `silent?` | `boolean` | Whether to suppress logging output (default: `true`) |

#### Returns

```typescript
interface RelayResult {
  blockHash: string;           // Hash of the block containing the transaction
  msgId: string;              // Message ID of the submitted transaction
  txHash: string;             // Hash of the Vara transaction
  isFinalized: Promise<boolean>; // Promise that resolves when transaction is finalized
  ok?: string;                // Success data if transaction succeeded
  error?: ProxyError;         // Error information if transaction failed
}
```

## Advanced Usage

### Error Handling

The library provides detailed error information through the `RelayResult` interface:

```typescript
const result = await relayEthToVaraTransaction(/* ... */);

if (result.error) {
  console.error('Transaction failed:', result.error);
} else {
  console.log('Transaction succeeded:', result.ok);

  // Wait for finalization
  const isFinalized = await result.isFinalized;
  console.log('Transaction finalized:', isFinalized);
}
```

### Working with Different Networks

The library supports various Ethereum networks. Configure your public client accordingly:

```typescript
// For Sepolia testnet
const ethereumClient = createPublicClient({
  chain: sepolia,
  transport: http('https://sepolia.infura.io/v3/your-project-id'),
});

// For Vara testnet
const gearApi = await GearApi.create({
  providerAddress: 'wss://testnet.vara.network',
});
```

## How It Works

The relay process involves several key steps:

1. **Transaction Receipt Retrieval**: Fetches the target transaction receipt from Ethereum
2. **Block Analysis**: Retrieves the block containing the transaction and all its receipts
3. **Slot Calculation**: Determines the corresponding beacon chain slot for the block
4. **Merkle Proof Generation**: Creates a cryptographic proof of transaction inclusion
5. **Inclusion Proof Building**: Constructs a proof linking the block to the beacon chain
6. **Vara Submission**: Submits the proof to Vara through the historical proxy program

## Dependencies

This library builds upon several key technologies:

- **Viem**: Ethereum client library for transaction handling
- **@gear-js/api**: Gear protocol API for Vara network interactions
- **@chainsafe/ssz**: SSZ serialization for beacon chain data
- **@ethereumjs/trie**: Merkle trie implementation for proof generation
- **@lodestar/types**: Ethereum consensus types for beacon chain integration

## Contributing

Contributions are welcome! Please ensure that:

1. All code follows TypeScript best practices with strict typing
2. New features include comprehensive JSDoc documentation
3. Tests are written for all new functionality
4. Code is properly formatted and linted

## Support

For questions and support:

- [GitHub Issues](https://github.com/gear-tech/gear-bridges/issues)
- [Gear Protocol Documentation](https://wiki.gear-tech.io/)
- [Vara Network](https://vara.network/)
