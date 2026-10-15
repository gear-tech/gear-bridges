import type { Transfer } from '../model/index.js';
import * as manager from './abi/erc20-manager.js';
import * as queue from './abi/message-queue.js';

export interface ReceiptLog {
  address: string;
  topics: string[];
  data: string;
  logIndex: string;
  transactionHash: string;
  blockHash: string;
  removed?: boolean;
}
export interface Receipt {
  transactionHash: string;
  blockHash: string;
  blockNumber: string;
  status: string;
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
