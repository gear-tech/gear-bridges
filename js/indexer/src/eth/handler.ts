import { randomUUID } from 'node:crypto';
import { isDeepStrictEqual } from 'node:util';
import { In, IsNull, MoreThan, Not } from 'typeorm';
import * as bridgingPayment from './abi/bridging-payment.js';
import { CompletedTransfer, Network, Status, Transfer } from '../model/index.js';
import * as erc20ManagerAbi from './abi/erc20-manager.js';
import * as messageQueueAbi from './abi/message-queue.js';
import { ethNonce, gearNonce } from '../common/index.js';
import type { Context } from './processor.js';
import { BatchState } from './batch-state.js';
import { config } from './config.js';
import { requireFinalizedBatch } from './finality.js';
import { matchesOutboundCompletion, Receipt } from './completion.js';

let pendingCompletionCursor: string | undefined;

async function reconcilePendingCompletions(ctx: Context): Promise<void> {
  // ponytail: cycle one bounded page per batch; use a dedicated worker if backlog latency matters.
  const load = () => ctx.store.find(CompletedTransfer, {
    where: { srcNetwork: Network.Vara, destNetwork: Network.Ethereum, matched: false,
      outboundReceipt: Not(IsNull()), ...(pendingCompletionCursor ? { id: MoreThan(pendingCompletionCursor) } : {}) },
    order: { id: 'ASC' }, take: 1000,
  });
  let pending = await load();
  if (!pending.length && pendingCompletionCursor) {
    pendingCompletionCursor = undefined;
    pending = await load();
  }
  if (!pending.length) return;
  const transfers = await ctx.store.find(Transfer, {
    where: { nonce: In(pending.map((row) => row.outboundReceipt!.nonce)), sourceNetwork: Network.Vara },
  });
  const byNonce = new Map<string, Transfer>();
  for (const transfer of transfers) {
    if (byNonce.has(transfer.nonce)) throw new Error('HOLD: ambiguous original Vara transfer nonce');
    byNonce.set(transfer.nonce, transfer);
  }
  const ready = pending.filter((row) => {
    const evidence = row.outboundReceipt!;
    if (evidence.queue !== config.msgQ || evidence.manager !== config.erc20Manager ||
        evidence.receipt.transactionHash.toLowerCase() !== row.txHash ||
        BigInt(evidence.receipt.blockNumber) !== BigInt(row.blockNumber) ||
        !Number.isSafeInteger(Number(row.blockNumber))) throw new Error('HOLD: pending outbound receipt identity changed');
    const transfer = byNonce.get(evidence.nonce);
    return transfer?.bridgingStartedAtBlock != null && !!transfer.ethBridgeBuiltInMsgHash;
  });
  await requireFinalizedBatch(ctx, ready.map((row) => ({
    height: Number(row.blockNumber), hash: row.outboundReceipt!.receipt.blockHash,
  })));
  const changed: Transfer[] = [];
  const matched: CompletedTransfer[] = [];
  for (const row of ready) {
    const evidence = row.outboundReceipt!, transfer = byNonce.get(evidence.nonce)!;
    if (!matchesOutboundCompletion(evidence.receipt, transfer, evidence.logIndex,
      { height: Number(row.blockNumber), hash: evidence.receipt.blockHash }, row.txHash, config.msgQ, config.erc20Manager)) continue;
    if (transfer.status === Status.Completed && transfer.completedAtTxHash?.toLowerCase() !== row.txHash) {
      throw new Error('HOLD: original Vara transfer already has a different completion');
    }
    transfer.status = Status.Completed;
    transfer.completedAt = row.timestamp;
    transfer.completedAtBlock = BigInt(row.blockNumber);
    transfer.completedAtTxHash = row.txHash;
    row.matched = true;
    changed.push(transfer);
    matched.push(row);
  }
  if (changed.length) await ctx.store.save(changed);
  if (matched.length) await ctx.store.save(matched);
  pendingCompletionCursor = pending[pending.length - 1].id;
}

export async function handleBatch(ctx: Context): Promise<void> {
  await requireFinalizedBatch(ctx);
  const state = new BatchState();
  await state.new(ctx);
  const receipts = new Map<string, Receipt>();
  const settled = new Map<string, Transfer>();
  const pending = new Map<string, CompletedTransfer>();
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
          let receipt = receipts.get(txHash);
          if (!receipt) {
            receipt = await ctx._chain.client.call<Receipt>('eth_getTransactionReceipt', [txHash]);
            if (!receipt) throw new Error('HOLD: original outbound receipt unavailable');
            if (receipt.transactionHash.toLowerCase() !== txHash ||
                receipt.blockHash.toLowerCase() !== block.header.hash.toLowerCase() ||
                BigInt(receipt.blockNumber) !== blockNumber) throw new Error('HOLD: original outbound receipt noncanonical');
            receipts.set(txHash, receipt);
          }
          if (!transfer || transfer.bridgingStartedAtBlock == null || !transfer.ethBridgeBuiltInMsgHash) {
            const originalLog = receipt.logs.find((entry) => Number(BigInt(entry.logIndex)) === log.logIndex);
            if (receipt.status !== '0x1' || !originalLog || originalLog.removed ||
                originalLog.transactionHash.toLowerCase() !== txHash || originalLog.blockHash.toLowerCase() !== block.header.hash.toLowerCase() ||
                originalLog.address.toLowerCase() !== address || originalLog.data.toLowerCase() !== log.data.toLowerCase() ||
                originalLog.topics.join(',').toLowerCase() !== log.topics.join(',').toLowerCase()) {
              throw new Error('HOLD: pending completion absent from canonical successful receipt');
            }
            const id = `outbound:${config.msgQ}:${gearNonce(nonce)}`;
            if (pending.has(id) && pending.get(id)!.txHash !== txHash) throw new Error('HOLD: conflicting pending outbound completion');
            pending.set(id, new CompletedTransfer({ id, srcNetwork: Network.Vara, destNetwork: Network.Ethereum,
              timestamp, blockNumber, txHash, matched: false,
              outboundReceipt: { receipt, logIndex: log.logIndex, nonce: gearNonce(nonce), queue: config.msgQ, manager: config.erc20Manager } }));
            continue;
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
  if (pending.size) {
    const existing = new Map((await ctx.store.find(CompletedTransfer, { where: { id: In([...pending.keys()]) } }))
      .map((row) => [row.id, row]));
    const added: CompletedTransfer[] = [];
    for (const row of pending.values()) {
      const prior = existing.get(row.id);
      if (prior) {
        if (prior.txHash !== row.txHash || BigInt(prior.blockNumber) !== row.blockNumber ||
            !isDeepStrictEqual(prior.outboundReceipt, row.outboundReceipt)) {
          throw new Error('HOLD: conflicting retained outbound completion');
        }
      } else added.push(row);
    }
    if (added.length) await ctx.store.save(added);
  }
  await reconcilePendingCompletions(ctx);
}
