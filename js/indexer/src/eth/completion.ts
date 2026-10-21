import { isDeepStrictEqual } from 'node:util';
import type { Transfer } from '../model/index.js';
import {
  checkpointId, createReceiptAuthenticator,
  type AgreedCheckpoint, type AuthenticatedReceipt, type EthereumEvidenceClients, type SourceReceiptAuth,
} from './finality.js';
import * as manager from './abi/erc20-manager.js';
import * as queue from './abi/message-queue.js';

export interface ReceiptLog {
  address: string;
  topics: string[];
  data: string;
  logIndex: string;
  transactionHash: string;
  blockHash: string;
  blockNumber: string;
  transactionIndex: string;
  removed?: boolean;
}
export interface Receipt {
  transactionHash: string;
  blockHash: string;
  blockNumber: string;
  status: string;
  transactionIndex: string;
  type: string;
  cumulativeGasUsed: string;
  logsBloom: string;
  logs: ReceiptLog[];
}

// The manager emits Bridged before the queue emits MessageProcessed. Delimit each
// effect by queue events so one Bridged cannot settle two messages in a multicall.
export function matchesOutboundCompletion(
  receipt: Receipt, transfer: Transfer, processedLogIndex: number,
  block: { height: number; hash: string }, txHash: string,
  queueAddress: string, managerAddress: string,
): boolean {
  if (receipt.status !== '0x1' || receipt.transactionHash.toLowerCase() !== txHash.toLowerCase() ||
      receipt.blockHash.toLowerCase() !== block.hash.toLowerCase() ||
      BigInt(receipt.blockNumber) !== BigInt(block.height) ||
      transfer.bridgingStartedAtBlock == null || !transfer.ethBridgeBuiltInMsgHash) return false;
  let previousIndex = -1;
  let processed: ReceiptLog | undefined;
  for (const log of receipt.logs) {
    if (log.removed || log.transactionHash.toLowerCase() !== txHash.toLowerCase() ||
        log.blockHash.toLowerCase() !== block.hash.toLowerCase()) return false;
    if (log.address.toLowerCase() !== queueAddress.toLowerCase() ||
        log.topics[0]?.toLowerCase() !== queue.events.MessageProcessed.topic.toLowerCase()) continue;
    const index = Number(BigInt(log.logIndex));
    if (index < processedLogIndex) previousIndex = Math.max(previousIndex, index);
    if (index === processedLogIndex) {
      if (processed) return false;
      processed = log;
    }
  }
  if (!processed) return false;
  const [sourceBlock, messageHash, nonce, destination] = queue.events.MessageProcessed.decode(processed);
  // The event height selects a stored root, which can include an earlier message.
  if (sourceBlock < BigInt(transfer.bridgingStartedAtBlock) || nonce.toString() !== transfer.nonce ||
      messageHash.toLowerCase() !== transfer.ethBridgeBuiltInMsgHash.toLowerCase() ||
      destination.toLowerCase() !== managerAddress.toLowerCase()) return false;
  const effects = receipt.logs.filter((log) => log.address.toLowerCase() === managerAddress.toLowerCase() &&
    log.topics[0]?.toLowerCase() === manager.events.Bridged.topic.toLowerCase() &&
    BigInt(log.logIndex) > BigInt(previousIndex) && BigInt(log.logIndex) < BigInt(processedLogIndex));
  if (effects.length !== 1) return false;
  const [sender, receiver, token, amount] = manager.events.Bridged.decode(effects[0]);
  return sender.toLowerCase() === transfer.sender.toLowerCase() &&
    receiver.toLowerCase() === transfer.receiver.toLowerCase() &&
    token.toLowerCase() === transfer.destination.toLowerCase() && amount === BigInt(transfer.amount);
}

export function qualifyOutboundCompletion(
  transfer: Transfer, original: AuthenticatedReceipt, queueAddress: string, managerAddress: string,
): { processedLogIndex: number } | undefined {
  const receipt = original.receipt;
  if (transfer.bridgingStartedAtBlock == null || !transfer.ethBridgeBuiltInMsgHash) return;
  const candidates = receipt.logs.filter((log) => {
    if (log.address !== queueAddress.toLowerCase() ||
        log.topics[0] !== queue.events.MessageProcessed.topic.toLowerCase()) return false;
    const [height, messageHash, nonce, destination] = queue.events.MessageProcessed.decode(log);
    return height >= BigInt(transfer.bridgingStartedAtBlock!) && nonce.toString() === transfer.nonce &&
      messageHash.toLowerCase() === transfer.ethBridgeBuiltInMsgHash!.toLowerCase() &&
      destination.toLowerCase() === managerAddress.toLowerCase();
  });
  if (candidates.length > 1) throw new Error('HOLD: ambiguous original outbound completion');
  if (!candidates.length) return;
  const processedLogIndex = Number(BigInt(candidates[0].logIndex));
  const height = Number(BigInt(receipt.blockNumber));
  if (!Number.isSafeInteger(processedLogIndex) || !Number.isSafeInteger(height)) {
    throw new Error('HOLD: completion coordinates exceed safe integer range');
  }
  return matchesOutboundCompletion(receipt, transfer, processedLogIndex,
    { height, hash: receipt.blockHash }, receipt.transactionHash, queueAddress, managerAddress)
    ? { processedLogIndex } : undefined;
}

// Old JSON is evidence, not a cache: absent newly required fields may be supplied,
// but every previously recorded decision field must still match the original.
export function assertRetainedReceipt(legacy: unknown, receipt: Receipt): void {
  if (!legacy || typeof legacy !== 'object' || Array.isArray(legacy)) {
    throw new Error('HOLD: missing retained original receipt');
  }
  const compare = (saved: Record<string, unknown>, current: Record<string, unknown>, fields: string[]) => {
    for (const key of fields) {
      if (!(key in saved)) continue;
      const value = saved[key];
      const expected = current[key];
      if (typeof expected === 'string') {
        if (typeof value !== 'string') throw new Error(`HOLD: malformed retained receipt ${key}`);
        const quantity = ['blockNumber', 'transactionIndex', 'logIndex', 'status', 'type', 'cumulativeGasUsed'].includes(key);
        if (quantity) {
          if (!/^0x[0-9a-f]+$/i.test(value) || BigInt(value) !== BigInt(expected)) {
            throw new Error(`HOLD: retained receipt ${key} changed`);
          }
        } else if (value.toLowerCase() !== expected) throw new Error(`HOLD: retained receipt ${key} changed`);
      } else if (key === 'topics') {
        if (!Array.isArray(value) || !isDeepStrictEqual(value.map((v) => typeof v === 'string' ? v.toLowerCase() : v), expected)) {
          throw new Error('HOLD: retained receipt topics changed');
        }
      } else if (value !== expected) throw new Error(`HOLD: retained receipt ${key} changed`);
    }
  };
  const saved = legacy as Record<string, unknown>;
  if (['transactionHash', 'blockHash', 'blockNumber', 'status'].some((key) => typeof saved[key] !== 'string')) {
    throw new Error('HOLD: retained original receipt identity is missing');
  }
  compare(saved, receipt as unknown as Record<string, unknown>,
    ['transactionHash', 'blockHash', 'blockNumber', 'transactionIndex', 'status', 'type', 'cumulativeGasUsed', 'logsBloom']);
  if (!Array.isArray(saved.logs) || saved.logs.length !== receipt.logs.length) throw new Error('HOLD: retained receipt logs changed');
  saved.logs.forEach((log, index) => {
    if (!log || typeof log !== 'object' || Array.isArray(log)) throw new Error('HOLD: malformed retained receipt log');
    compare(log as Record<string, unknown>, { ...receipt.logs[index], removed: false },
      ['address', 'topics', 'data', 'logIndex', 'transactionHash', 'blockHash', 'blockNumber', 'transactionIndex', 'removed']);
  });
}

export async function authenticateSourceReceipt(
  clients: EthereumEvidenceClients, transfer: Transfer, managerAddress: string, checkpoint: AgreedCheckpoint,
  authenticate = createReceiptAuthenticator(clients, checkpoint),
): Promise<SourceReceiptAuth | undefined> {
  const retained = transfer.sourceReceiptAuth;
  const pending = () => {
    if (retained || transfer.completionAuth) throw new Error('HOLD: previously authenticated source receipt unavailable or changed');
    return undefined;
  };
  if (transfer.sourceTransactionIndex == null || transfer.sourceLogIndex == null || !transfer.txHash) return pending();
  const height = Number(transfer.blockNumber);
  const localLogIndex = Number(transfer.sourceLogIndex);
  if (!Number.isSafeInteger(height) || height < 0 || !Number.isSafeInteger(localLogIndex) || localLogIndex < 0) {
    throw new Error('HOLD: source receipt coordinates exceed safe integer range');
  }
  const original = await authenticate(transfer.txHash, { height, ...(retained ? { hash: retained.receipt.blockHash } : {}) });
  if (!original) return pending();
  const receipt = original.receipt;
  if (receipt.status !== '0x1' || BigInt(receipt.transactionIndex) !== BigInt(transfer.sourceTransactionIndex)) return pending();
  const log = receipt.logs[localLogIndex];
  if (!log || log.address !== managerAddress.toLowerCase() ||
      log.topics[0] !== manager.events.BridgingRequested.topic.toLowerCase()) return pending();
  const [sender, receiver, token, amount] = manager.events.BridgingRequested.decode(log);
  if (sender.toLowerCase() !== transfer.sender.toLowerCase() || receiver.toLowerCase() !== transfer.receiver.toLowerCase() ||
      token.toLowerCase() !== transfer.source.toLowerCase() || amount !== BigInt(transfer.amount)) return pending();
  if (retained) {
    if (retained.version !== 1 || retained.localLogIndex !== localLogIndex ||
        retained.blockTimestamp !== original.blockTimestamp || !retained.checkpointId) {
      throw new Error('HOLD: retained source authorization changed');
    }
    if (!isDeepStrictEqual(retained.receipt, receipt)) throw new Error('HOLD: retained authenticated source receipt changed');
    return retained;
  }
  return { version: 1, checkpointId: checkpointId(checkpoint), receipt, blockTimestamp: original.blockTimestamp, localLogIndex };
}
