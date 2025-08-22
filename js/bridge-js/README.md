# @gear-js/bridge

A TypeScript library for relaying transactions between Ethereum and Vara networks.

## Overview

The `@gear-js/bridge` library provides a seamless way to create bidirectional cross-chain bridges between Ethereum and Vara networks. It enables developers to relay transactions in both directions by generating cryptographic proofs of transaction inclusion and submitting them to the target network.

## Installation

```bash
npm install @gear-js/bridge
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
import { relayEthToVara, relayVaraToEth } from '@gear-js/bridge';
import { createPublicClient, createWalletClient, http } from 'viem';
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
const result = await relayEthToVara(
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

// Or relay from Vara to Ethereum
const varaToEthResult = await relayVaraToEth(
  123n, // Message nonce
  12345n, // Vara block number
  ethereumClient, // Public client
  walletClient, // Wallet client
  account, // Ethereum account
  gearApi, // Gear API instance
  '0x1234...', // Message queue contract address
  false, // Enable logging
);

console.log('Vara to Ethereum transaction processed');
```

## API Reference

### `relayEthToVara`

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

### `relayVaraToEth`

Relays a queued message from Vara network to Ethereum by finding the message, generating merkle proof, and processing it through the message queue contract.

#### Parameters

| Parameter | Type | Description |
|-----------|------|-------------|
| `nonce` | `bigint \| HexString` | The message nonce to relay (little endian encoded if hex string) |
| `blockNumber` | `bigint` | The Vara block number containing the message |
| `ethereumPublicClient` | `PublicClient` | Ethereum public client for reading blockchain state |
| `ethereumWalletClient` | `WalletClient` | Ethereum wallet client for sending transactions |
| `ethereumAccount` | `Account` | Ethereum account to use for transactions |
| `gearApi` | `GearApi` | Gear API instance for interacting with Vara network |
| `messageQueueAddress` | `0x${string}` | Ethereum message queue contract address |
| `silent?` | `boolean` | Whether to suppress logging output (default: `true`) |

#### Returns

```typescript
Promise<void> // Resolves when the message is successfully processed
```

#### Throws

- **Error**: If the message with the given nonce is not found in the specified block

#### Example

```typescript
import { relayVaraToEth } from '@gear-js/bridge';
import { createPublicClient, createWalletClient, http } from 'viem';
import { mainnet } from 'viem/chains';
import { privateKeyToAccount } from 'viem/accounts';

const publicClient = createPublicClient({
  chain: mainnet,
  transport: http(),
});

const account = privateKeyToAccount('0x...');
const walletClient = createWalletClient({
  account,
  chain: mainnet,
  transport: http(),
});

await relayVaraToEth(
  42n, // Message nonce
  1000000n, // Block number
  publicClient,
  walletClient,
  account,
  gearApi,
  '0x1234567890123456789012345678901234567890',
);
```

### `waitForMerkleRootAppearedInMessageQueue`

Waits for a Merkle root to appear in the message queue contract for the specified block number or greater.

#### Parameters

| Parameter | Type | Description |
|-----------|------|-------------|
| `blockNumber` | `bigint` | The block number to wait for the Merkle root |
| `publicClient` | `PublicClient` | Ethereum public client for reading blockchain state |
| `messageQueueAddress` | `0x${string}` | The message queue contract address |

#### Returns

```typescript
Promise<boolean> // Resolves to true when the Merkle root appears for the specified block or a block greater than specified
```

## Advanced Usage

### Error Handling

The library provides detailed error information through the `RelayResult` interface:

```typescript
const result = await relayEthToVara(/* ... */);

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

The library supports bidirectional relaying with different processes for each direction:

### Ethereum to Vara Relay Process

1. **Transaction Receipt Retrieval**: Fetches the target transaction receipt from Ethereum
2. **Block Analysis**: Retrieves the block containing the transaction and all its receipts
3. **Slot Calculation**: Determines the corresponding beacon chain slot for the block
4. **Merkle Proof Generation**: Creates a cryptographic proof of transaction inclusion
5. **Inclusion Proof Building**: Constructs a proof linking the block to the beacon chain
6. **Vara Submission**: Submits the proof to Vara through the historical proxy program

### Vara to Ethereum Relay Process

1. **Message Retrieval**: Finds the queued message in the specified Vara block using the nonce
2. **Authority Set Verification**: Retrieves the authority set ID for the block to ensure validity
3. **Merkle Root Resolution**: Obtains the Merkle root from Ethereum message queue or searches in recent blocks
4. **Proof Generation**: Creates a Merkle proof for the message inclusion in the Vara block
5. **Ethereum Submission**: Processes the message through the Ethereum message queue contract

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
