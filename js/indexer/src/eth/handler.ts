import { randomUUID } from 'node:crypto';
import { isDeepStrictEqual } from 'node:util';
import { In, IsNull, MoreThan, Not } from 'typeorm';
import * as bridgingPayment from './abi/bridging-payment.js';
import { CompletedTransfer, EthereumFinalityCheckpoint, Network, Status, Transfer } from '../model/index.js';
import * as erc20ManagerAbi from './abi/erc20-manager.js';
import * as messageQueueAbi from './abi/message-queue.js';
import { ethNonce, gearNonce } from '../common/index.js';
import type { Context } from './processor.js';
import { BatchState } from './batch-state.js';
import { config } from './config.js';
import { createEthereumEvidenceClients, createReceiptAuthenticator, recheckFinality, receiptDate,
  receiptAuthorization, proofReference, assertReceiptAuthorization, assertAuthorizationCheckpoint, requalificationLinksFor,
  requireFinalizedBatch, retainCheckpoint, type EthereumEvidenceClients, type AuthenticatedReceipt } from './finality.js';
import { assertRetainedReceipt, qualifyOutboundCompletion } from './completion.js';

let pendingCompletionCursor: string | undefined;
let pendingCompletionBatch: string | undefined;
let pendingCompletionStart: string | undefined;


function bindObservation(
  clients: EthereumEvidenceClients, row: CompletedTransfer, original: AuthenticatedReceipt,
): void {
  if (!row.outboundReceipt) throw new Error('HOLD: missing retained outbound receipt');
  if (row.srcNetwork !== Network.Vara || row.destNetwork !== Network.Ethereum ||
      !Number.isSafeInteger(row.outboundReceipt.logIndex) || row.outboundReceipt.logIndex < 0) {
    throw new Error('HOLD: malformed retained outbound completion');
  }
  if (row.outboundReceipt.queue !== config.msgQ || row.outboundReceipt.manager !== config.erc20Manager ||
      row.txHash.toLowerCase() !== original.receipt.transactionHash || BigInt(row.blockNumber) !== BigInt(original.receipt.blockNumber)) {
    throw new Error('HOLD: retained outbound completion identity changed');
  }
  assertRetainedReceipt(row.outboundReceipt.receipt, original.receipt);
  const archived = row.legacyReceiptAuth;
  if (archived) {
    assertAuthorizationCheckpoint(clients, archived.receiptAuth.checkpointId, BigInt(row.blockNumber));
    assertRetainedReceipt(archived.receiptAuth.receipt, original.receipt);
    if (archived.receiptAuth.version !== 1 || archived.txHash.toLowerCase() !== row.txHash.toLowerCase() ||
        BigInt(archived.blockNumber) !== BigInt(row.blockNumber) ||
        archived.receiptAuth.blockTimestamp !== original.blockTimestamp || !original.requalificationLinks.length) {
      throw new Error('HOLD: archived completion lacks exact Beacon requalification');
    }
  }
  const requalified = requalificationLinksFor(clients, original,
    [archived?.receiptAuth.checkpointId], row.receiptAuth?.requalificationLinks);
  if (row.receiptAuth) {
    assertReceiptAuthorization(clients, row.receiptAuth, original);
  } else row.receiptAuth = receiptAuthorization(original, requalified);
}

function settle(clients: EthereumEvidenceClients, transfer: Transfer, row: CompletedTransfer, original: AuthenticatedReceipt): Transfer | undefined {
  const match = qualifyOutboundCompletion(transfer, original, config.msgQ, config.erc20Manager);
  if (!match || match.processedLogIndex !== row.outboundReceipt!.logIndex) {
    if (transfer.completionAuth || row.matched) throw new Error('HOLD: previously qualified outbound effect changed');
    return;
  }
  const legacy = transfer.legacyEthereumAuth ?? transfer.legacyResolution;
  if ((legacy?.completedAtTxHash && legacy.completedAtTxHash.toLowerCase() !== row.txHash.toLowerCase()) ||
      (legacy?.completedAtBlock != null && BigInt(legacy.completedAtBlock) !== BigInt(row.blockNumber))) {
    throw new Error('HOLD: archived terminal original cannot be replaced');
  }
  const archived = transfer.legacyEthereumAuth?.completionAuth;
  if (archived) {
    if (archived.kind !== 'ethereum-receipt' || !archived.checkpointId || archived.completionId !== row.id ||
        archived.processedLogIndex !== match.processedLogIndex) {
      throw new Error('HOLD: archived completion binding changed');
    }
    assertAuthorizationCheckpoint(clients, archived.checkpointId, BigInt(row.blockNumber));
  }
  const auth = transfer.completionAuth;
  if (auth && auth.kind !== 'ethereum-receipt') throw new Error('HOLD: original outbound completion has another evidence kind');
  const requalified = requalificationLinksFor(clients, original, [archived?.checkpointId], auth?.requalificationLinks);
  // The observation can predate its counterpart; preserve its authorization separately.
  const authorization = auth ?? receiptAuthorization(original, requalified);
  assertReceiptAuthorization(clients, { ...row.receiptAuth!, ...authorization }, original);
  const timestamp = receiptDate(original.blockTimestamp);
  if (auth) {
    if (auth.version !== 2 || auth.completionId !== row.id ||
        auth.processedLogIndex !== match.processedLogIndex ||
        transfer.status !== Status.Completed || transfer.completedAt?.getTime() !== timestamp.getTime() ||
        transfer.completedAtBlock == null || BigInt(transfer.completedAtBlock) !== BigInt(row.blockNumber) ||
        transfer.completedAtTxHash?.toLowerCase() !== row.txHash.toLowerCase()) {
      throw new Error('HOLD: original outbound terminal binding cannot be replaced');
    }
  } else if ([Status.Completed, Status.Refunded, Status.Failed].includes(transfer.status)) {
    throw new Error('HOLD: unauthenticated terminal outbound projection; migrate before processing');
  }
  row.matched = true;
  return auth ? undefined : new Transfer({ ...transfer, status: Status.Completed, completedAt: timestamp,
    completedAtBlock: BigInt(row.blockNumber), completedAtTxHash: row.txHash,
    completionAuth: { version: 2, kind: 'ethereum-receipt', checkpointId: authorization.checkpointId,
      completionId: row.id, processedLogIndex: match.processedLogIndex, ...proofReference(authorization) } });
}

export async function handleBatch(ctx: Context): Promise<void> {
  const batchKey = ctx.blocks.map(({ header }) => `${header.height}:${header.hash}`).join(',');
  // A serializable retry receives the same batch. Reuse its starting cursor,
  // not the tentative cursor from a callback whose COMMIT may have failed.
  if (pendingCompletionBatch !== batchKey) {
    pendingCompletionBatch = batchKey;
    pendingCompletionStart = pendingCompletionCursor;
  }
  const checkpoint = await requireFinalizedBatch(ctx);
  const clients = createEthereumEvidenceClients('eth');
  const authenticate = createReceiptAuthenticator(clients, checkpoint);
  const state = new BatchState();
  await state.new(ctx);
  const observations = new Map<string, CompletedTransfer>();
  const originals = new Map<string, AuthenticatedReceipt>();
  for (const block of ctx.blocks) {
    for (const event of block.logs) {
      const address = event.address.toLowerCase();
      const topic = event.topics[0]?.toLowerCase();
      const consumed = (address === config.erc20Manager && topic === erc20ManagerAbi.events.BridgingRequested.topic) ||
        (address === config.msgQ && [messageQueueAbi.events.MessageProcessed.topic, messageQueueAbi.events.MerkleRoot.topic].includes(topic)) ||
        (address === config.bridgingPayment && topic === bridgingPayment.events.FeePaid.topic);
      if (!consumed) continue;
      const txHash = event.transactionHash.toLowerCase();
      const original = await authenticate(txHash, { height: block.header.height, hash: block.header.hash });
      if (!original || original.receipt.status !== '0x1') throw new Error('HOLD: consumed event lacks a finalized successful receipt');
      const receipt = original.receipt;
      const localIndex = receipt.logs.findIndex((entry) => BigInt(entry.logIndex) === BigInt(event.logIndex));
      const log = receipt.logs[localIndex];
      if (!log || log.address !== address || log.data !== event.data.toLowerCase() ||
          !isDeepStrictEqual(log.topics, event.topics.map((value) => value.toLowerCase())) ||
          BigInt(receipt.transactionIndex) !== BigInt(event.transactionIndex)) {
        throw new Error('HOLD: consumed event absent from independently qualified receipt');
      }
      const timestamp = receiptDate(original.blockTimestamp);
      const blockNumber = BigInt(receipt.blockNumber);
      if (address === config.erc20Manager) {
        const [from, to, token, amount] = erc20ManagerAbi.events.BridgingRequested.decode(log);
        const firstDeposit = !receipt.logs.slice(0, localIndex).some((entry) => entry.address === config.erc20Manager &&
          entry.topics[0] === erc20ManagerAbi.events.BridgingRequested.topic);
        const transactionIndex = BigInt(receipt.transactionIndex);
        await state.addTransfer(new Transfer({
          id: randomUUID(), txHash, blockNumber, timestamp,
          nonce: ethNonce(firstDeposit ? `${blockNumber}${transactionIndex}` : `${blockNumber}:${transactionIndex}:${localIndex}`),
          sourceTransactionIndex: transactionIndex, sourceLogIndex: BigInt(localIndex),
          sourceReceiptAuth: { ...receiptAuthorization(original), localLogIndex: localIndex },
          sourceNetwork: Network.Ethereum, source: token, destNetwork: Network.Vara,
          status: Status.AwaitingPayment, sender: from, receiver: to, amount: amount.toString(),
        }));
      } else if (address === config.msgQ && topic === messageQueueAbi.events.MessageProcessed.topic) {
        const [, , nonce, receiver] = messageQueueAbi.events.MessageProcessed.decode(log);
        if (receiver.toLowerCase() !== config.erc20Manager) continue;
        const id = `outbound:${config.msgQ}:${gearNonce(nonce)}`;
        const index = Number(BigInt(log.logIndex));
        if (!Number.isSafeInteger(index)) throw new Error('HOLD: completion log index out of range');
        const existing = observations.get(id);
        if (existing && (existing.txHash !== txHash || existing.outboundReceipt!.logIndex !== index)) {
          throw new Error('HOLD: conflicting pending outbound completion');
        }
        const row = new CompletedTransfer({ id, srcNetwork: Network.Vara, destNetwork: Network.Ethereum,
          timestamp, blockNumber, txHash, matched: false,
          outboundReceipt: { receipt, logIndex: index, nonce: gearNonce(nonce), queue: config.msgQ, manager: config.erc20Manager } });
        observations.set(id, row);
        originals.set(id, original);
      } else if (address === config.msgQ) {
        const [height, root] = messageQueueAbi.events.MerkleRoot.decode(log);
        state.newMerkleRoot(height, root, blockNumber, txHash);
      } else state.bridgingPaid(txHash);
    }
  }

  // Repair owns migrated unauthenticated history (legacyMatched can be true or false).
  // Exclude it before pagination so it cannot starve current qualified observations.
  let cursor = pendingCompletionStart;
  const load = () => ctx.store.find(CompletedTransfer, {
    where: [{ legacyMatched: IsNull() }, { receiptAuth: Not(IsNull()) }].map((evidence) => ({
      srcNetwork: Network.Vara, destNetwork: Network.Ethereum, matched: false,
      outboundReceipt: Not(IsNull()), ...(cursor ? { id: MoreThan(cursor) } : {}), ...evidence,
    })),
    order: { id: 'ASC' }, take: 1000,
  });
  let pending = await load();
  if (!pending.length && cursor) { cursor = undefined; pending = await load(); }
  const retained = observations.size ? await ctx.store.find(CompletedTransfer, { where: { id: In([...observations.keys()]) } }) : [];
  for (const prior of retained) {
    const current = observations.get(prior.id)!;
    if (!prior.outboundReceipt || prior.txHash.toLowerCase() !== current.txHash ||
        BigInt(prior.blockNumber) !== BigInt(current.blockNumber) || prior.outboundReceipt.logIndex !== current.outboundReceipt!.logIndex ||
        prior.outboundReceipt.nonce !== current.outboundReceipt!.nonce) throw new Error('HOLD: conflicting retained outbound completion');
    observations.set(prior.id, new CompletedTransfer(prior));
  }
  for (const row of pending) if (!observations.has(row.id)) observations.set(row.id, new CompletedTransfer(row));
  const transfers = observations.size ? await ctx.store.find(Transfer, {
    where: { nonce: In([...observations.values()].map((row) => row.outboundReceipt!.nonce)), sourceNetwork: Network.Vara },
  }) : [];
  const byNonce = new Map<string, Transfer>();
  for (const transfer of transfers) {
    if (byNonce.has(transfer.nonce)) throw new Error('HOLD: ambiguous original Vara transfer nonce');
    byNonce.set(transfer.nonce, transfer);
  }
  const savedCheckpointIds = [...new Set([
    ...[...observations.values()].map((row) => row.receiptAuth?.checkpointId),
    ...transfers.map((transfer) => transfer.completionAuth?.kind === 'ethereum-receipt' ? transfer.completionAuth.checkpointId : undefined),
  ].filter((id): id is string => id !== undefined))];
  if (savedCheckpointIds.length) {
    const rows = await ctx.store.find(EthereumFinalityCheckpoint, { where: { id: In(savedCheckpointIds) } });
    const existing = new Map(rows.map((row) => [row.id, row]));
    if (savedCheckpointIds.some((id) => !existing.has(id))) throw new Error('HOLD: outbound authorization checkpoint is missing');
    for (const row of observations.values()) {
      if (row.receiptAuth && BigInt(existing.get(row.receiptAuth.checkpointId)!.checkpoint.number) < BigInt(row.blockNumber)) {
        throw new Error('HOLD: retained checkpoint does not cover original completion');
      }
    }
  }
  const updates: Transfer[] = [];
  for (const row of observations.values()) {
    const height = Number(row.blockNumber);
    if (!Number.isSafeInteger(height) || height < 0) throw new Error('HOLD: completion block out of range');
    const original = row.receiptAuth
      ? await authenticate(row.txHash, { height, hash: row.outboundReceipt!.receipt.blockHash, proofDigest: row.receiptAuth.inclusionProofDigest })
      : originals.get(row.id) ?? await authenticate(row.txHash, { height, hash: row.outboundReceipt!.receipt.blockHash });
    if (!original) throw new Error('HOLD: retained completion receipt unavailable');
    const transfer = byNonce.get(row.outboundReceipt!.nonce);
    bindObservation(clients, row, original);
    if (!transfer) {
      if (row.matched) throw new Error('HOLD: matched completion lost its original transfer');
      continue;
    }
    const update = settle(clients, transfer, row, original);
    if (update) updates.push(update);
  }
  await recheckFinality(clients, checkpoint);
  await retainCheckpoint(ctx.store, checkpoint);
  await state.save();
  if (updates.length) await ctx.store.save(updates);
  if (observations.size) await ctx.store.save([...observations.values()]);
  pendingCompletionCursor = pending.length ? pending[pending.length - 1].id : cursor;
}
