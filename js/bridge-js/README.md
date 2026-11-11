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
   - Historical proxy program

## Choose one bridge deployment

Vara Mainnet is the only supported public Substrate deployment. Its paired network
is Ethereum Mainnet (chain ID `1`), not Sepolia or Hoodi. For a production integration,
use `wss://rpc.vara.network`, an Ethereum Mainnet execution RPC and a Mainnet beacon
RPC, together with the addresses in the
[bridge deployment list](https://wiki.vara.network/docs/vara-network/bridge/developer_hub).
Verify the current on-chain configuration before submitting a relay. Do not mix
addresses from different deployments. Vara.eth/Hoodi is a separate environment;
it is not a replacement RPC for a Substrate `GearApi` client.

For learning to sign and submit transactions, use the local development scenario
below. Full cross-chain examples additionally need a coordinated bridge fixture.

## Relay integration

This illustrates the current object-parameter API. The clients, signer, transaction,
block, nonce and program/contract IDs must all come from your verified deployment;
the snippet does not provision a bridge or create a test transaction.

```typescript
import { relayEthToVara, relayVaraToEth } from '@gear-js/bridge';

// Relay transaction
const result = await relayEthToVara({
  transactionHash,
  beaconRpcUrl,
  ethereumPublicClient: ethereumClient,
  gearApi,
  historicalProxyId,
  clientId,
  clientServiceName: 'MyService',
  clientMethodName: 'MyMethod',
  signer: signerAccount,
});

console.log('Transaction relayed:', result.txHash);

// Or relay from Vara to Ethereum
await relayVaraToEth({
  nonce,
  blockNumber,
  ethereumPublicClient: ethereumClient,
  ethereumWalletClient: walletClient,
  ethereumAccount: account,
  gearApi,
  messageQueueAddress,
});

console.log('Vara to Ethereum transaction processed');
```

## API Reference

### `relayEthToVara`

Relays an Ethereum transaction to the Vara network by creating a proof and submitting it through the historical proxy program.

Accepts a single parameter object with the following fields.

#### Parameters

| Parameter | Type | Description |
|-----------|------|-------------|
| `transactionHash` | `0x${string}` | Transaction hash of the Ethereum transaction to relay |
| `beaconRpcUrl` | `string` | RPC URL for the Ethereum beacon chain client |
| `ethereumPublicClient` | `PublicClient` | Viem public client for Ethereum network interactions |
| `gearApi` | `GearApi` | Gear API instance for Vara network operations |
| `historicalProxyId` | `0x${string}` | ID of the historical proxy program on Vara |
| `clientId` | `0x${string}` | ID of the target client program on Vara |
| `clientServiceName` | `string` | Name of the service to call on the target client |
| `clientMethodName` | `string` | Name of the method to call on the target service |
| `signer` | `string \| KeyringPair` | Account signer for transaction signing |
| `signerOptions?` | `Partial<SignerOptions>` | Optional signing configuration parameters |
| `wait?` | `boolean` | Whether to wait for the checkpoint required by the proof |
| `statusCb?` | `(status, details?) => void` | Optional progress callback |

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

Accepts a single parameter object with the following fields.

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
| `wait?` | `boolean` | Whether to wait for the required Merkle root |
| `statusCb?` | `(status, details?) => void` | Optional progress callback |

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
import { privateKeyToAccount } from 'viem/accounts';

// chain, RPC and MessageQueue must come from the same deployment manifest.
const publicClient = createPublicClient({
  chain,
  transport: http(executionRpcUrl),
});

const account = privateKeyToAccount('0x...');
const walletClient = createWalletClient({
  account,
  chain,
  transport: http(executionRpcUrl),
});

await relayVaraToEth({
  nonce,
  blockNumber,
  ethereumPublicClient: publicClient,
  ethereumWalletClient: walletClient,
  ethereumAccount: account,
  gearApi,
  messageQueueAddress,
});
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

### Local development writes

Run a supported Gear node in development mode, as described in the
[node guide](https://wiki.vara.network/docs/vara-network/node):

```sh
gear --dev --tmp
```

Connect to its local WebSocket RPC and submit a balance transfer with development
accounts. This needs no bridge contracts, archive, beacon RPC or public faucet:

```typescript
import { GearApi, GearKeyring } from '@gear-js/api';

const gearApi = await GearApi.create({
  providerAddress: 'ws://127.0.0.1:9944',
});
const alice = await GearKeyring.fromSuri('//Alice');
const bob = await GearKeyring.fromSuri('//Bob');

const tx = gearApi.balance.transfer(bob.address, 1_000_000_000_000n);
try {
  await new Promise<void>((resolve, reject) => {
    let unsubscribe: (() => void) | undefined;
    tx.signAndSend(alice, ({ status, dispatchError }) => {
      if (dispatchError) {
        unsubscribe?.();
        reject(new Error(dispatchError.toString()));
      } else if (status.isFinalized) {
        console.log('Finalized in block:', status.asFinalized.toHex());
        unsubscribe?.();
        resolve();
      }
    })
      .then((unsub) => {
        unsubscribe = unsub;
      })
      .catch(reject);
  });
} finally {
  await gearApi.disconnect();
}
```

Development keys are for your local chain. The public Vara Explorer indexes
Mainnet; inspect local blocks with Gear IDEA or Polkadot.js configured for your
local RPC.

### Full bridge examples and CI fixtures

The scripts in `example/` are full relay demonstrations, not a local-node setup.
They use the development signer `//Alice`. Before running them, provision a
bridge-compatible Gear development chain, checkpoint/eth-events/historical-proxy
programs, an Ethereum MessageQueue/target contract, and the core relayers. See
[running the bridge](../../docs/running-the-bridge.md). A plain `gear --dev` node
alone cannot provide cross-chain proofs or relayed roots.

Supply one fixture manifest with `VARA_WS_RPC`, `ETH_RPC_URL`, `BEACON_RPC_URL`,
`HISTORICAL_PROXY_ID`, `MESSAGE_QUEUE_ADDRESS`, `ETH_DESTINATION` and a development
`ETH_PRIVATE_KEY`. Program IDs must be the actual IDs deployed on that Gear chain;
Ethereum addresses, execution/beacon RPCs and the checkpoint client's consensus
configuration must describe the same Ethereum chain. There are no universal local
contract addresses. Keep keys in an untracked `.env` in the package working
directory. Do not use production funds for these demonstrations.

The Ethereum-to-Vara example explicitly uses the Hoodi chain definition. Keep its
execution and beacon clients on Hoodi and deploy/configure the Gear-side fixture
to verify Hoodi. It is not a public Substrate test network or a Vara.eth deployment.
For a fully local Ethereum fixture, adapt the chain definition and consensus
fixture together; an execution-only local node cannot replace a beacon chain.

CI uses the `bridge-js-test` GitHub environment and its RPC/program secrets. Audit
those external values before running the integration suite. Historical transaction
fixtures must be read from the chain where they were recorded; do not retarget
their block numbers, nonces or hashes to a new chain. Hoodi and Vara.eth deployment
files elsewhere in this repository remain independent.

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
