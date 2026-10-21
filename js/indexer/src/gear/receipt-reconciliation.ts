import type { Store } from '@subsquid/typeorm-store';
import { In } from 'typeorm';
import { EthereumFinalityCheckpoint, Network, ReceiptRelay, ReceiptSettlement, Status, Transfer } from '../model/index.js';
import type { SourceReceiptAuth } from '../eth/finality.js';
import type { ReceiptDepositEffect } from './types/index.js';

export function validateReceiptSettlement(deposit: ReceiptDepositEffect): void {
  if (!('outcome' in deposit)) return;
  if (deposit.outcome !== 'Refunded') throw new Error('HOLD: unsupported receipt settlement outcome');
  const messageId = (id: string) => /^0x[0-9a-fA-F]{64}$/.test(id) && BigInt(id) !== 0n;
  if (!deposit.native || !messageId(deposit.operation_id) || !messageId(deposit.child) || !messageId(deposit.payout_child) ||
      deposit.child === deposit.payout_child || BigInt(deposit.returned_value) !== BigInt(deposit.amount)) {
    throw new Error('HOLD: native refund lacks its original children or full returned backing');
  }
}

export async function reconcileReceiptSettlements(
  store: Store, pending: readonly ReceiptSettlement[],
  authenticateSource: (transfer: Transfer) => Promise<SourceReceiptAuth | undefined>,
): Promise<{ transfers: Transfer[]; settlements: ReceiptSettlement[] }> {
  if (!pending.length) return { transfers: [], settlements: [] };
  const receiptIds = [...new Set(pending.map(({ deposit: d }) => `${d.slot}:${d.transaction_index}`))];
  const relays = new Map((await store.find(ReceiptRelay, { where: { id: In(receiptIds) } })).map((row) => [row.id, row]));
  const tuples = pending.flatMap(({ deposit: d }) => {
    const relay = relays.get(`${d.slot}:${d.transaction_index}`);
    return relay ? [{ sourceNetwork: Network.Ethereum, blockNumber: relay.blockNumber,
      sourceTransactionIndex: BigInt(d.transaction_index), sourceLogIndex: BigInt(d.log_index) }] : [];
  });
  const originals = tuples.length ? await store.find(Transfer, { where: tuples }) : [];
  const transfersByTuple = new Map<string, Transfer>();
  for (const transfer of originals) {
    const key = `${transfer.blockNumber}:${transfer.sourceTransactionIndex}:${transfer.sourceLogIndex}`;
    if (transfersByTuple.has(key)) throw new Error('HOLD: ambiguous original receipt transfer tuple');
    transfersByTuple.set(key, transfer);
  }
  const changed = new Map<string, Transfer>();
  const settlements: ReceiptSettlement[] = [];
  const checkpointIds = [...new Set(originals.flatMap((transfer) => [
    transfer.sourceReceiptAuth?.checkpointId,
    transfer.completionAuth?.kind === 'gear-settlement' ? transfer.completionAuth.sourceCheckpointId : undefined,
  ].filter((id): id is string => id !== undefined)))];
  if (checkpointIds.length) {
    const retained = new Map((await store.find(EthereumFinalityCheckpoint, { where: { id: In(checkpointIds) } })).map((row) => [row.id, row]));
    if (checkpointIds.some((id) => !retained.has(id))) throw new Error('HOLD: source authorization checkpoint is missing');
    for (const transfer of originals) {
      if (transfer.sourceReceiptAuth &&
          BigInt(retained.get(transfer.sourceReceiptAuth.checkpointId)!.checkpoint.number) < BigInt(transfer.blockNumber)) {
        throw new Error('HOLD: retained checkpoint does not cover original source receipt');
      }
    }
  }
  for (const settlement of pending) {
    const d = settlement.deposit;
    validateReceiptSettlement(d);
    const relayId = `${d.slot}:${d.transaction_index}`;
    const relay = relays.get(relayId);
    if (!relay) {
      if (settlement.matched) throw new Error('HOLD: matched settlement lost its original receipt relay');
      continue;
    }
    const original = transfersByTuple.get(`${relay.blockNumber}:${BigInt(d.transaction_index)}:${BigInt(d.log_index)}`);
    if (!original) {
      if (settlement.matched) throw new Error('HOLD: matched settlement lost its original transfer');
      continue;
    }
    const transfer = changed.get(original.id) ?? new Transfer(original);
    const conflict = () => {
      if (transfer.completionAuth || settlement.matched) throw new Error('HOLD: authenticated receipt settlement effect changed');
    };
    if (transfer.status === Status.Failed || transfer.source.toLowerCase() !== d.eth_token_id.toLowerCase() ||
        transfer.destination.toLowerCase() !== d.vara_token_id.toLowerCase() ||
        transfer.sender.toLowerCase() !== d.sender.toLowerCase() || transfer.receiver.toLowerCase() !== d.receiver.toLowerCase() ||
        BigInt(transfer.amount) !== BigInt(d.amount)) { conflict(); continue; }
    if (transfer.receiptSlot != null && BigInt(transfer.receiptSlot) !== BigInt(d.slot)) {
      throw new Error('HOLD: original transfer already bound to another receipt slot');
    }
    const source = await authenticateSource(transfer);
    if (!source) { conflict(); continue; }
    const outcome = 'outcome' in d && d.outcome === 'Refunded' ? Status.Refunded : Status.Completed;
    const auth = transfer.completionAuth;
    if (auth) {
      if (auth.version !== 1 || auth.kind !== 'gear-settlement' || auth.sourceCheckpointId !== source.checkpointId ||
          auth.receiptRelayId !== relayId || auth.receiptSettlementId !== settlement.id || auth.outcome !== outcome ||
          transfer.status !== outcome || transfer.completedAt?.getTime() !== settlement.timestamp.getTime() ||
          transfer.completedAtBlock == null || BigInt(transfer.completedAtBlock) !== BigInt(settlement.blockNumber) ||
          transfer.completedAtTxHash?.toLowerCase() !== settlement.txHash.toLowerCase()) {
        throw new Error('HOLD: original receipt terminal authorization cannot be replaced');
      }
    } else {
      if (transfer.status === Status.Completed || transfer.status === Status.Refunded) {
        throw new Error('HOLD: terminal receipt projection lacks authentication; migrate before processing');
      }
      transfer.status = outcome;
      transfer.completedAt = settlement.timestamp;
      transfer.completedAtBlock = settlement.blockNumber;
      transfer.completedAtTxHash = settlement.txHash;
      transfer.completionAuth = { version: 1, kind: 'gear-settlement', sourceCheckpointId: source.checkpointId,
        receiptRelayId: relayId, receiptSettlementId: settlement.id, outcome };
    }
    transfer.sourceReceiptAuth = source;
    transfer.receiptSlot = BigInt(d.slot);
    changed.set(transfer.id, transfer);
    settlements.push(new ReceiptSettlement({ ...settlement, matched: true }));
  }
  return { transfers: [...changed.values()], settlements };
}
