import { Account, PublicClient, WalletClient, WatchContractEventReturnType, zeroHash, decodeEventLog } from 'viem';
import { bytesToHex } from '@ethereumjs/util';
import { bnToU8a } from '@polkadot/util';

import { MerkleRootLogArgs, MessageProcessResult } from './types.js';
import { Proof, VaraMessage } from '../vara/types.js';
import { logger } from '../util.js';
import { bytesToBigint } from 'viem/utils';

const MerkleRootEventAbi = [
  {
    type: 'event',
    name: 'MerkleRoot',
    inputs: [
      { name: 'blockNumber', type: 'uint256', indexed: false, internalType: 'uint256' },
      { name: 'merkleRoot', type: 'bytes32', indexed: false, internalType: 'bytes32' },
    ],
    anonymous: false,
  },
] as const;

export const MessageQueueAbi = [
  {
    type: 'function',
    name: 'processMessage',
    inputs: [
      { name: 'blockNumber', type: 'uint256', internalType: 'uint256' },
      { name: 'totalLeaves', type: 'uint256', internalType: 'uint256' },
      { name: 'leafIndex', type: 'uint256', internalType: 'uint256' },
      {
        name: 'message',
        type: 'tuple',
        internalType: 'struct VaraMessage',
        components: [
          { name: 'nonce', type: 'uint256', internalType: 'uint256' },
          { name: 'source', type: 'bytes32', internalType: 'bytes32' },
          { name: 'destination', type: 'address', internalType: 'address' },
          { name: 'payload', type: 'bytes', internalType: 'bytes' },
        ],
      },
      { name: 'proof', type: 'bytes32[]', internalType: 'bytes32[]' },
    ],
    outputs: [],
    stateMutability: 'nonpayable',
  },
  {
    type: 'function',
    name: 'getMerkleRoot',
    inputs: [{ name: 'blockNumber', type: 'uint256', internalType: 'uint256' }],
    outputs: [{ name: '', type: 'bytes32', internalType: 'bytes32' }],
    stateMutability: 'view',
  },
  {
    type: 'function',
    name: 'isProcessed',
    inputs: [{ name: 'messageNonce', type: 'uint256', internalType: 'uint256' }],
    outputs: [{ name: '', type: 'bool', internalType: 'bool' }],
    stateMutability: 'view',
  },
  { type: 'error', name: 'EmergencyStop', inputs: [] },
  { type: 'error', name: 'InvalidMerkleProof', inputs: [] },
  { type: 'error', name: 'InvalidPlonkProof', inputs: [] },
  {
    type: 'error',
    name: 'MerkleRootAlreadySet',
    inputs: [{ name: 'blockNumber', type: 'uint256', internalType: 'uint256' }],
  },
  {
    type: 'error',
    name: 'MerkleRootNotFound',
    inputs: [{ name: 'blockNumber', type: 'uint256', internalType: 'uint256' }],
  },
  {
    type: 'error',
    name: 'MessageAlreadyProcessed',
    inputs: [{ name: 'messageNonce', type: 'uint256', internalType: 'uint256' }],
  },
  {
    type: 'event',
    name: 'MessageProcessed',
    inputs: [
      { name: 'blockNumber', type: 'uint256', indexed: false, internalType: 'uint256' },
      { name: 'messageHash', type: 'bytes32', indexed: false, internalType: 'bytes32' },
      { name: 'messageNonce', type: 'uint256', indexed: false, internalType: 'uint256' },
      { name: 'messageDestination', type: 'address', indexed: false, internalType: 'address' },
    ],
    anonymous: false,
  },
] as const;

type MerkleProofArgs = [
  bigint,
  bigint,
  bigint,
  {
    nonce: bigint;
    destination: `0x${string}`;
    source: `0x${string}`;
    payload: `0x${string}`;
  },
  `0x${string}`[],
];

export const getProcessMessageArgs = (blockNumber: bigint, varaMessage: VaraMessage, proof: Proof): MerkleProofArgs => {
  return [
    blockNumber,
    proof.numLeaves,
    proof.leafIndex,
    {
      nonce: bytesToBigint(bnToU8a(varaMessage.nonce, { bitLength: 256, isLe: true })),
      destination: bytesToHex(varaMessage.destination),
      source: bytesToHex(varaMessage.source),
      payload: bytesToHex(varaMessage.payload),
    },
    proof.proof,
  ];
};

export class MessageQueueClient {
  constructor(
    private _address: `0x${string}`,
    private _client: PublicClient,
    private _walletClient?: WalletClient,
    private _account?: Account,
  ) {}

  public async getMerkleRoot(blockNumber: bigint): Promise<`0x${string}` | null> {
    const result = (await this._client.readContract({
      address: this._address,
      abi: MessageQueueAbi,
      functionName: 'getMerkleRoot',
      args: [blockNumber],
    })) as `0x${string}`;

    if (result === zeroHash) {
      return null;
    } else {
      return result;
    }
  }

  public async waitForMekleRoot(bn: bigint): Promise<MerkleRootLogArgs> {
    let unwatch: WatchContractEventReturnType;

    const result = await new Promise<MerkleRootLogArgs>((resolve, reject) => {
      unwatch = this._client.watchContractEvent({
        address: this._address,
        abi: MerkleRootEventAbi,
        eventName: 'MerkleRoot',
        onLogs: (logs) => {
          for (const log of logs) {
            if ('args' in log) {
              const { blockNumber, merkleRoot } = log.args as MerkleRootLogArgs;
              logger.info(`Received merkle root ${merkleRoot} for block ${blockNumber}`);
              if (blockNumber >= bn) {
                return resolve({ blockNumber, merkleRoot });
              }
            }
          }
        },
        onError: (error) => {
          reject(error);
        },
      });
    });

    if (unwatch! !== undefined) {
      unwatch();
    }

    return result;
  }

  async getMerkleRootLogsInRange(fromBlock: bigint, toBlock: bigint) {
    const filter = await this._client.createEventFilter({
      address: this._address,
      event: MerkleRootEventAbi[0],
      fromBlock,
      toBlock,
    });

    const logs = await this._client.getFilterLogs({
      filter,
    });

    return logs;
  }

  async findMerkleRootInRangeOfBlocks(
    fromBlock: bigint,
    toBlock: bigint,
    targetBlockNumber: bigint,
  ): Promise<MerkleRootLogArgs> {
    const logs = await this.getMerkleRootLogsInRange(fromBlock, toBlock);

    if (logs.length === 0) {
      throw new Error(`No merkle root logs found in range ${fromBlock} to ${toBlock}`);
    }

    const log = logs.find(({ args: { blockNumber } }) => blockNumber === targetBlockNumber);

    if (log) {
      return log.args as MerkleRootLogArgs;
    }

    const eligibleLogs = logs.filter(({ args: { blockNumber } }) => blockNumber! > targetBlockNumber);

    if (eligibleLogs.length === 0) {
      throw new Error(`No merkle root logs found with blockNumber greater than or equal to ${targetBlockNumber}`);
    }

    const closestLog = eligibleLogs.reduce((closest, current) => {
      const closestDiff = closest.args.blockNumber! - targetBlockNumber;
      const currentDiff = current.args.blockNumber! - targetBlockNumber;
      return currentDiff < closestDiff ? current : closest;
    });

    return closestLog.args as MerkleRootLogArgs;
  }

  async processMessage(
    blockNumber: bigint,
    varaMessage: VaraMessage,
    merkleProof: Proof,
  ): Promise<MessageProcessResult> {
    if (!this._walletClient || !this._account) {
      throw new Error('Wallet client must be provided');
    }
    try {
      const { request } = await this._client.simulateContract({
        address: this._address,
        abi: MessageQueueAbi,
        functionName: 'processMessage',
        args: getProcessMessageArgs(blockNumber, varaMessage, merkleProof),
        account: this._account,
      });

      logger.info(
        `Sending processMessage transaction ${JSON.stringify(request.args, (_, value) => {
          if (typeof value === 'bigint') {
            return value.toString();
          }
          return value;
        })}`,
      );

      const hash = await this._walletClient.writeContract({
        address: this._address,
        abi: MessageQueueAbi,
        functionName: 'processMessage',
        args: getProcessMessageArgs(blockNumber, varaMessage, merkleProof),
        account: this._account,
        chain: this._walletClient.chain,
      });

      logger.info(`Waiting for transaction receipt ${hash}`);

      const receipt = await this._client.waitForTransactionReceipt({ hash });

      logger.info(`Transaction receipt received ${hash}`);

      const messageProcessedLog = receipt.logs.find((log) => {
        try {
          const decodedLog = decodeEventLog({
            abi: MessageQueueAbi,
            eventName: 'MessageProcessed',
            data: log.data,
            topics: log.topics,
          });
          return decodedLog.eventName === 'MessageProcessed';
        } catch {
          return false;
        }
      });

      if (messageProcessedLog) {
        const { args } = decodeEventLog({
          abi: MessageQueueAbi,
          eventName: 'MessageProcessed',
          data: messageProcessedLog.data,
          topics: messageProcessedLog.topics,
        });

        logger.info(`Message processed event received ${hash}`);

        return {
          success: true,
          transactionHash: hash,
          blockNumber: args.blockNumber,
          messageHash: args.messageHash,
          messageNonce: args.messageNonce,
          messageDestination: args.messageDestination,
        };
      } else {
        logger.info(`Message processed event not found in transaction receipt ${hash}`);
        return {
          success: false,
          transactionHash: hash,
          error: 'MessageProcessed event not found in transaction receipt',
        };
      }
    } catch (error: any) {
      logger.error(
        `Error processing message queue transaction ${error}. args: ${JSON.stringify(error.args, (_, value) => {
          if (typeof value === 'bigint') {
            return value.toString();
          }
          return value;
        })}`,
      );
      return {
        success: false,
        transactionHash: '0x' as `0x${string}`,
        error: error instanceof Error ? error.message : 'Unknown error occurred',
      };
    }
  }
}

export function getMessageQueueClient(
  address: `0x${string}`,
  publicClient: PublicClient,
  walletClient?: WalletClient,
  account?: Account,
) {
  return new MessageQueueClient(address, publicClient, walletClient, account);
}

/**
 * Waits for a Merkle root to appear in the message queue contract for the specified block number or greater.
 *
 * @param blockNumber - The block number to wait for the Merkle root
 * @param publicClient - Ethereum public client for reading blockchain state
 * @param messageQueueAddress - The message queue contract address
 * @returns Promise that resolves to true when the Merkle root appears for the specified block or a block greater than specified
 */
export async function waitForMerkleRootAppearedInMessageQueue(
  blockNumber: bigint,
  publicClient: PublicClient,
  messageQueueAddress: `0x${string}`,
): Promise<boolean> {
  const client = getMessageQueueClient(messageQueueAddress, publicClient);
  await client.waitForMekleRoot(blockNumber);
  return true;
}
