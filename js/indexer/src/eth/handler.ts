import { randomUUID } from 'node:crypto';
import * as bridgingPayment from './abi/bridging-payment.js';
import { Network, Status, Transfer } from '../model/index.js';
import * as erc20ManagerAbi from './abi/erc20-manager.js';
import * as messageQueueAbi from './abi/message-queue.js';
import { ethNonce, gearNonce } from '../common/index.js';
import type { Context } from './processor.js';
import { BatchState } from './batch-state.js';
import { config } from './config.js';
import { requireFinalizedBatch } from './finality.js';
import { matchesOutboundCompletion, Receipt } from './completion.js';

export async function handleBatch(ctx: Context): Promise<void> {
  await requireFinalizedBatch(ctx);
  const state = new BatchState();
  await state.new(ctx);
  const receipts = new Map<string, Receipt>();
  const settled = new Map<string, Transfer>();
  for (const block of ctx.blocks) {
    const timestamp = new Date(block.header.timestamp);
    const blockNumber = BigInt(block.header.height);
    for (const log of block.logs) {
      const address = log.address.toLowerCase();
      const topic = log.topics[0].toLowerCase();
      const txHash = log.transactionHash.toLowerCase();
      if (address === config.erc20Manager && topic === erc20ManagerAbi.events.BridgingRequested.topic) {
        let receipt = receipts.get(txHash);
        if (!receipt) {
          receipt = await ctx._chain.client.call<Receipt>('eth_getTransactionReceipt', [txHash]);
          if (!receipt || receipt.status !== '0x1' || receipt.transactionHash.toLowerCase() !== txHash ||
              receipt.blockHash.toLowerCase() !== block.header.hash.toLowerCase() ||
              BigInt(receipt.blockNumber) !== blockNumber) throw new Error('HOLD: original deposit receipt unavailable/noncanonical');
          receipts.set(txHash, receipt);
        }
        const localIndex = receipt.logs.findIndex((entry) => Number(BigInt(entry.logIndex)) === log.logIndex &&
          !entry.removed && entry.address.toLowerCase() === address && entry.data === log.data &&
          entry.topics.join(',').toLowerCase() === log.topics.join(',').toLowerCase());
        if (localIndex < 0) throw new Error('HOLD: original deposit absent from canonical receipt');
        const [from, to, token, amount] = erc20ManagerAbi.events.BridgingRequested.decode(log);
        // Preserve the original transaction nonce for its first deposit; later logs
        // need separate identities rather than overwriting the first transfer.
        const firstDeposit = !receipt.logs.slice(0, localIndex).some((entry) =>
          entry.address.toLowerCase() === config.erc20Manager &&
          entry.topics[0]?.toLowerCase() === erc20ManagerAbi.events.BridgingRequested.topic.toLowerCase());
        await state.addTransfer(new Transfer({
          id: randomUUID(), txHash, blockNumber, timestamp,
          nonce: ethNonce(firstDeposit ? `${block.header.height}${log.transactionIndex}` :
            `${block.header.height}:${log.transactionIndex}:${localIndex}`),
          sourceTransactionIndex: BigInt(log.transactionIndex), sourceLogIndex: BigInt(localIndex),
          sourceNetwork: Network.Ethereum, source: token, destNetwork: Network.Vara,
          status: Status.AwaitingPayment, sender: from, receiver: to, amount: amount.toString(),
        }));
      } else if (address === config.msgQ) {
        if (topic === messageQueueAbi.events.MessageProcessed.topic) {
          const [, , nonce, receiver] = messageQueueAbi.events.MessageProcessed.decode(log);
          if (receiver.toLowerCase() !== config.erc20Manager) continue;
          const transfer = settled.get(gearNonce(nonce)) ?? await ctx.store.findOneBy(Transfer, {
            nonce: gearNonce(nonce), sourceNetwork: Network.Vara,
          });
          if (!transfer) throw new Error(`HOLD: original Vara transfer ${nonce} not indexed`);
          if (transfer.bridgingStartedAtBlock == null || !transfer.ethBridgeBuiltInMsgHash) {
            throw new Error(`HOLD: original Vara message ${nonce} identity not indexed`);
          }
          let receipt = receipts.get(txHash);
          if (!receipt) {
            receipt = await ctx._chain.client.call<Receipt>('eth_getTransactionReceipt', [txHash]);
            if (!receipt) throw new Error('HOLD: original outbound receipt unavailable');
            if (receipt.transactionHash.toLowerCase() !== txHash ||
                receipt.blockHash.toLowerCase() !== block.header.hash.toLowerCase() ||
                BigInt(receipt.blockNumber) !== blockNumber) throw new Error('HOLD: original outbound receipt noncanonical');
            receipts.set(txHash, receipt);
          }
          if (matchesOutboundCompletion(receipt, transfer, log.logIndex, block.header, txHash, config.msgQ, config.erc20Manager)) {
            transfer.status = Status.Completed;
            transfer.completedAt = timestamp;
            transfer.completedAtBlock = blockNumber;
            transfer.completedAtTxHash = txHash;
            settled.set(transfer.nonce, transfer);
          } else {
            ctx.log.warn({ nonce: transfer.nonce, txHash }, 'Token effect unmatched; transfer remains pending');
          }
        } else if (topic === messageQueueAbi.events.MerkleRoot.topic) {
          const [merkleRootBlockNumber, merkleRoot] = messageQueueAbi.events.MerkleRoot.decode(log);
          state.newMerkleRoot(merkleRootBlockNumber, merkleRoot, blockNumber, txHash);
        }
      } else if (address === config.bridgingPayment && topic === bridgingPayment.events.FeePaid.topic) {
        state.bridgingPaid(txHash);
      }
    }
  }
  await state.save();
  if (settled.size) await ctx.store.save([...settled.values()]);
}
