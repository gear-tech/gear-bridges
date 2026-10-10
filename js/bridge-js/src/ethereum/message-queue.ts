import { Account, PublicClient, WalletClient, TransactionSerializable, zeroHash, parseEventLogs, encodeFunctionData, TransactionReceiptNotFoundError } from 'viem';
import { bytesToHex } from '@ethereumjs/util';
import { HexString } from '@gear-js/api';

import { MerkleRootLog, MerkleRootLogArgs, MessageProcessResult } from './types.js';
import { Proof, VaraMessage } from '../vara/types.js';
import { StatusCb, withOriginalDeadline } from '../util.js';
import { messageHash } from '../vara-to-eth/util.js';

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
      nonce: varaMessage.nonce,
      destination: bytesToHex(varaMessage.destination),
      source: bytesToHex(varaMessage.source),
      payload: bytesToHex(varaMessage.payload),
    },
    proof.proof,
  ];
};

export const BridgedEventAbi = [{ type: 'event', name: 'Bridged', anonymous: false, inputs: [
  { name: 'from', type: 'bytes32', indexed: true }, { name: 'to', type: 'address', indexed: true },
  { name: 'token', type: 'address', indexed: true }, { name: 'amount', type: 'uint256', indexed: false },
] }] as const;

export type OutboundEffect = { readonly kind: 'application' } | {
  readonly kind: 'token'; readonly managerAddress: HexString; readonly sourceActorId: HexString;
  readonly token: HexString; readonly sender: HexString; readonly receiver: HexString; readonly amount: bigint;
};

function assertTokenMessage(message: VaraMessage, effect: OutboundEffect): void {
  if (!effect || !['application', 'token'].includes(effect.kind)) throw new Error('HOLD: an explicit expected outbound effect is required');
  if (effect.kind !== 'token') return;
  const payload = effect.sender + effect.receiver.slice(2) + effect.token.slice(2) + effect.amount.toString(16).padStart(64, '0');
  if (effect.amount <= 0n || effect.amount >= 1n << 256n ||
      bytesToHex(message.source).toLowerCase() !== effect.sourceActorId.toLowerCase() ||
      bytesToHex(message.destination).toLowerCase() !== effect.managerAddress.toLowerCase() ||
      bytesToHex(message.payload).toLowerCase() !== payload.toLowerCase()) {
    throw new Error('Original token message does not match the expected source and packed economic effect');
  }
}

export type FinalizedMessageReceiptParams = {
  ethereumPublicClient: PublicClient;
  messageQueueAddress: HexString;
  transactionHash: HexString;
  blockNumber: bigint;
  message: VaraMessage;
  proof: Proof;
  sender: HexString;
  deadline: number;
  expectedEffect: OutboundEffect;
  statusCb?: StatusCb;
};

export type FinalizedMessageProcessResult = MessageProcessResult & {
  success: true;
  receiptBlockHash: HexString;
  receiptBlockNumber: bigint;
  transactionIndex: number;
};

/** Shared by fresh submissions and original-intent resume; never accepts a replacement transaction. */
export function validateFinalizedMessageReceipt(params: FinalizedMessageReceiptParams): Promise<FinalizedMessageProcessResult> {
  return withOriginalDeadline(params.deadline, () => validateOriginalFinalizedMessageReceipt(params));
}

async function validateOriginalFinalizedMessageReceipt(params: FinalizedMessageReceiptParams): Promise<FinalizedMessageProcessResult> {
  const { ethereumPublicClient: client, messageQueueAddress: address, transactionHash: hash, blockNumber, message, proof,
    sender, deadline } = params;
  const input = encodeFunctionData({ abi: MessageQueueAbi, functionName: 'processMessage',
    args: getProcessMessageArgs(blockNumber, message, proof) });
  assertTokenMessage(message, params.expectedEffect);
  while (Date.now() < deadline) {
    let receipt;
    try {
      receipt = await client.getTransactionReceipt({ hash });
    } catch (error) {
      if (!(error instanceof TransactionReceiptNotFoundError)) throw error;
    }
    if (receipt) {
      if (receipt.transactionHash.toLowerCase() !== hash.toLowerCase()) throw new Error('Original receipt transaction hash mismatch');
      const finalized = await client.getBlock({ blockTag: 'finalized' });
      if (receipt.blockNumber <= finalized.number) {
        const canonical = await client.getBlock({ blockNumber: receipt.blockNumber });
        if (canonical.hash !== receipt.blockHash) throw new Error('Original receipt is not canonical finalized history');
        if (receipt.status !== 'success') throw new Error('Original finalized processMessage transaction reverted');
        const transaction = await client.getTransaction({ hash });
        if (transaction.hash.toLowerCase() !== hash.toLowerCase() || transaction.blockHash !== receipt.blockHash ||
            transaction.blockNumber !== receipt.blockNumber || transaction.transactionIndex !== receipt.transactionIndex ||
            transaction.to?.toLowerCase() !== address.toLowerCase() || transaction.from.toLowerCase() !== sender.toLowerCase() ||
            transaction.input.toLowerCase() !== input.toLowerCase() || transaction.value !== 0n) {
          throw new Error('Original finalized transaction does not match the submitted queue call');
        }
        const events = parseEventLogs({ abi: MessageQueueAbi, eventName: 'MessageProcessed', strict: true,
          logs: receipt.logs.filter((log) => log.address.toLowerCase() === address.toLowerCase()) });
        if (events.length !== 1) throw new Error('Expected exactly one configured queue MessageProcessed event');
        const [event] = events;
        const expectedHash = messageHash(message);
        const { args } = event;
        if (event.removed || event.blockHash !== receipt.blockHash || event.blockNumber !== receipt.blockNumber ||
            event.transactionHash?.toLowerCase() !== hash.toLowerCase() || event.transactionIndex !== receipt.transactionIndex ||
            args.blockNumber !== blockNumber || args.messageNonce !== message.nonce ||
            args.messageDestination.toLowerCase() !== bytesToHex(message.destination).toLowerCase() ||
            args.messageHash.toLowerCase() !== expectedHash.toLowerCase()) {
          throw new Error('Configured queue MessageProcessed does not match the original authenticated message');
        }
        if (params.expectedEffect.kind === 'token') {
          const effect = params.expectedEffect;
          const transfers = parseEventLogs({ abi: BridgedEventAbi, eventName: 'Bridged', strict: true,
            logs: receipt.logs.filter(log => log.address.toLowerCase() === effect.managerAddress.toLowerCase()) });
          if (transfers.length !== 1) throw new Error('Expected exactly one same-receipt ERC20Manager.Bridged event');
          const transfer = transfers[0];
          if (transfer.removed || transfer.blockHash !== receipt.blockHash || transfer.blockNumber !== receipt.blockNumber ||
              transfer.transactionHash?.toLowerCase() !== hash.toLowerCase() || transfer.transactionIndex !== receipt.transactionIndex ||
              transfer.args.from.toLowerCase() !== effect.sender.toLowerCase() || transfer.args.to.toLowerCase() !== effect.receiver.toLowerCase() ||
              transfer.args.token.toLowerCase() !== effect.token.toLowerCase() || transfer.args.amount !== effect.amount) {
            throw new Error('Same-receipt ERC20Manager.Bridged differs from the original token effect');
          }
        }
        const [processed, root] = await Promise.all([
          client.readContract({ address, abi: MessageQueueAbi, functionName: 'isProcessed', args: [message.nonce],
            blockNumber: receipt.blockNumber }),
          client.readContract({ address, abi: MessageQueueAbi, functionName: 'getMerkleRoot', args: [blockNumber],
            blockNumber: receipt.blockNumber }),
        ]);
        if (processed !== true || root === zeroHash || root.toLowerCase() !== proof.root.toLowerCase()) {
          throw new Error('Original finalized queue state does not prove processed nonce and selected stored root');
        }
        if ((await client.getBlock({ blockNumber: receipt.blockNumber })).hash !== receipt.blockHash) {
          throw new Error('Original finalized receipt pin changed during queue readback');
        }
        params.statusCb?.('Original processMessage receipt finalized', { txHash: hash, receiptBlockHash: receipt.blockHash });
        return { success: true, transactionHash: hash, ...args, receiptBlockHash: receipt.blockHash,
          receiptBlockNumber: receipt.blockNumber, transactionIndex: receipt.transactionIndex };
      }
    }
    await new Promise((resolve) => setTimeout(resolve, Math.max(1, Math.min(client.pollingInterval, deadline - Date.now()))));
  }
  throw new Error('Original relay deadline expired without a canonical finalized processMessage receipt');
}

export class MessageQueueClient {
  constructor(
    private _address: `0x${string}`,
    private _client: PublicClient,
    private _walletClient?: WalletClient,
    private _account?: Account,
  ) {}

  public async getMerkleRoot(blockNumber: bigint, ethereumBlockNumber?: bigint): Promise<HexString | null> {
    const result = await this._client.readContract({
      address: this._address, abi: MessageQueueAbi, functionName: 'getMerkleRoot', args: [blockNumber],
      ...(ethereumBlockNumber === undefined ? { blockTag: 'finalized' as const } : { blockNumber: ethereumBlockNumber }),
    });
    return result === zeroHash ? null : result;
  }

  public async waitForMerkleRoot(
    bn: bigint,
    fromBlock?: bigint,
    statusCb: StatusCb = () => {},
  ): Promise<MerkleRootLogArgs> {
    const merkleRoot = await this.getMerkleRoot(bn);
    if (merkleRoot) return { blockNumber: bn, merkleRoot };

    const latestBlock = await this._client.getBlockNumber();

    return new Promise<MerkleRootLogArgs>((resolve, reject) => {
      statusCb(`Subscribing to merkle root events`);
      const unwatch = this._client.watchContractEvent({
        address: this._address,
        abi: MerkleRootEventAbi,
        eventName: 'MerkleRoot',
        fromBlock: fromBlock ?? latestBlock,
        onLogs: (logs) => {
          for (const log of logs) {
            if ('args' in log) {
              const { blockNumber, merkleRoot } = log.args as MerkleRootLogArgs;
              statusCb(`Received merkle root`, { blockNumber: blockNumber.toString(), merkleRoot });
              if (blockNumber >= bn) {
                unwatch();
                return resolve({ blockNumber, merkleRoot });
              }
            }
          }
        },
        onError: (error) => {
          if (unwatch) {
            unwatch();
          }
          reject(error);
        },
      });
    });
  }

  async getMerkleRootLogsInRange(fromBlock: bigint, toBlock: bigint) {
    return this._client.getLogs({ address: this._address, event: MerkleRootEventAbi[0], fromBlock, toBlock, strict: true });
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

    const log = logs.find(({ args: { blockNumber } }) => blockNumber === targetBlockNumber) as MerkleRootLog;

    if (log) {
      return log.args;
    }

    const eligibleLogs = logs.filter(
      ({ args: { blockNumber } }) => blockNumber! > targetBlockNumber,
    ) as MerkleRootLog[];

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
    expectedEffect: OutboundEffect,
    statusCb: StatusCb = () => {},
    deadline = Date.now() + 44 * 60 * 1000,
  ): Promise<MessageProcessResult> {
    const wallet = this._walletClient, account = this._account;
    if (!wallet || !account) throw new Error('Wallet client must be provided');
    let hash: HexString = '0x';
    let submissionStarted = false;
    try {
      if (!Number.isSafeInteger(deadline) || Date.now() >= deadline) throw new Error('Original relay deadline expired');
      const args = getProcessMessageArgs(blockNumber, varaMessage, merkleProof);
      assertTokenMessage(varaMessage, expectedEffect);
      await this._client.simulateContract({ address: this._address, abi: MessageQueueAbi, functionName: 'processMessage',
        args, account: this._account });
      statusCb('Sending processMessage transaction');
      if (Date.now() >= deadline) throw new Error('Original relay deadline expired before broadcasting');
      if (account.type === 'local') {
        const request = await withOriginalDeadline(deadline, () => wallet.prepareTransactionRequest({
          account, to: this._address, data: encodeFunctionData({ abi: MessageQueueAbi,
            functionName: 'processMessage', args }), value: 0n, chain: wallet.chain,
        }));
        const signed = await withOriginalDeadline(deadline, () => account.signTransaction(request as TransactionSerializable,
          { serializer: wallet.chain?.serializers?.transaction }));
        if (Date.now() >= deadline) throw new Error('Original relay deadline expired before raw submission');
        submissionStarted = true;
        hash = await withOriginalDeadline(deadline, () => wallet.sendRawTransaction({ serializedTransaction: signed }));
      } else {
        submissionStarted = true;
        hash = await withOriginalDeadline(deadline, () => wallet.writeContract({ address: this._address,
          abi: MessageQueueAbi, functionName: 'processMessage', args, account, chain: wallet.chain }));
      }
      statusCb('Waiting for original finalized processMessage receipt', { txHash: hash });
      return await validateFinalizedMessageReceipt({ ethereumPublicClient: this._client, messageQueueAddress: this._address,
        transactionHash: hash, blockNumber, message: varaMessage, proof: merkleProof, sender: account.address,
        deadline, statusCb, expectedEffect });
    } catch (error) {
      const message = error instanceof Error ? error.message : 'Unknown error processing original message';
      const reason = submissionStarted && hash === '0x'
        ? 'HOLD: original wallet submission outcome unresolved; it may still broadcast; reconcile the original account/nonce. ' + message
        : message;
      statusCb('Original message not proven finalized', { txHash: hash, error: reason });
      return { success: false, transactionHash: hash, error: reason };
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
 * @param fromEthereumBlock - (optional) The block number to start searching for the Merkle root
 * @returns Promise that resolves to true when the Merkle root appears for the specified block or a block greater than specified
 */
export async function waitForMerkleRootAppearedInMessageQueue(
  blockNumber: bigint,
  publicClient: PublicClient,
  messageQueueAddress: `0x${string}`,
  fromEthereumBlock?: bigint,
  statusCb: StatusCb = () => {},
): Promise<boolean> {
  const client = getMessageQueueClient(messageQueueAddress, publicClient);
  await client.waitForMerkleRoot(blockNumber, fromEthereumBlock, statusCb);
  return true;
}
