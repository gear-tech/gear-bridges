import 'reflect-metadata';
import { pathToFileURL } from 'node:url';
import { isDeepStrictEqual } from 'node:util';
import { DataSource, In, MoreThan, Not } from 'typeorm';
import { Store } from '@subsquid/typeorm-store';
import { CompletedTransfer, EthereumFinalityCheckpoint, Network, ReceiptRelay, ReceiptSettlement, Status, Transfer } from '../model/index.js';
import existing from '../model/dataSource.js';
import { reconcileReceiptSettlements } from '../gear/receipt-reconciliation.js';
import { assertRetainedReceipt, authenticateSourceReceipt, qualifyOutboundCompletion } from './completion.js';
import {
  checkpointId, createEthereumEvidenceClients, createReceiptAuthenticator, destroyEthereumEvidenceClients,
  loadCheckpoint, qualifyFinality, retainCheckpoint,
  type AgreedCheckpoint, type EthereumEvidenceClients, type ReceiptAuthenticator,
} from './finality.js';

type Direction = 'vara-to-eth' | 'eth-to-vara';
export interface ReconcileOptions { direction: Direction; limit: number; after?: string; apply: boolean }
export interface ReconcileReport {
  direction: Direction;
  limit: number;
  apply: boolean;
  lastExaminedId: string | null;
  hasMore: boolean;
  qualified: number;
  pending: number;
  heldIds: string[];
}

export function parseReconcileArgs(args: readonly string[]): ReconcileOptions {
  let direction: Direction | undefined, after: string | undefined, limit = 1000, apply = false;
  const seen = new Set<string>();
  for (let i = 0; i < args.length; i++) {
    const flag = args[i];
    if (!['--direction', '--limit', '--after', '--apply'].includes(flag) || seen.has(flag)) {
      throw new Error(`Unknown or duplicate argument: ${flag}`);
    }
    seen.add(flag);
    if (flag === '--apply') { apply = true; continue; }
    const value = args[++i];
    if (!value || value.startsWith('--')) throw new Error(`Missing value for ${flag}`);
    if (flag === '--direction') {
      if (value !== 'vara-to-eth' && value !== 'eth-to-vara') throw new Error('Invalid --direction');
      direction = value;
    } else if (flag === '--limit') {
      if (!/^[1-9][0-9]{0,3}$/.test(value) || value.trim() !== value || Number(value) > 1000) throw new Error('Invalid --limit: expected 1..1000');
      limit = Number(value);
    } else {
      // Transfer IDs are Gear message hashes or generated UUIDs, not arbitrary SQL/cursor text.
      if (value.trim() !== value || !/^(?:0x[0-9a-f]{64}|[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12})$/.test(value)) {
        throw new Error('Invalid --after transfer ID');
      }
      after = value;
    }
  }
  if (!direction) throw new Error('Required --direction vara-to-eth|eth-to-vara');
  return { direction, limit, ...(after === undefined ? {} : { after }), apply };
}

function height(value: bigint | string | number): number {
  const n = BigInt(value);
  if (n < 0n || n > BigInt(Number.MAX_SAFE_INTEGER)) throw new Error('HOLD: original block height is out of range');
  return Number(n);
}

function receiptDate(seconds: string): Date {
  const millis = BigInt(seconds) * 1000n;
  if (millis < 0n || millis > 8640000000000000n) throw new Error('HOLD: receipt timestamp is out of range');
  return new Date(Number(millis));
}

function sameCoordinates(transfer: Transfer, timestamp: Date, blockNumber: bigint, txHash: string): boolean {
  return transfer.completedAt?.getTime() === timestamp.getTime() && transfer.completedAtBlock != null &&
    BigInt(transfer.completedAtBlock) === blockNumber && transfer.completedAtTxHash?.toLowerCase() === txHash;
}

function referenceCheckpoint(references: Map<string, bigint>, id: string, block: bigint): void {
  const height = BigInt(block);
  if (!id || height < 0n) throw new Error('HOLD: malformed original checkpoint reference');
  if (!references.has(id) || references.get(id)! < height) references.set(id, height);
}

// The caller owns the SERIALIZABLE transaction. All writes are staged until every
// original and the fixed page checkpoint have been requalified.
export async function reconcilePage(
  store: Store, clients: EthereumEvidenceClients, options: ReconcileOptions, queue: string, manager: string,
): Promise<ReconcileReport> {
  const selected = await store.find(Transfer, {
    where: { sourceNetwork: options.direction === 'vara-to-eth' ? Network.Vara : Network.Ethereum,
      destNetwork: options.direction === 'vara-to-eth' ? Network.Ethereum : Network.Vara,
      status: Not(Status.Failed), ...(options.after ? { id: MoreThan(options.after) } : {}) },
    order: { id: 'ASC' }, take: options.limit + 1,
  });
  const page = selected.slice(0, options.limit);
  const report: ReconcileReport = { direction: options.direction, limit: options.limit, apply: options.apply,
    lastExaminedId: page.at(-1)?.id ?? null, hasMore: selected.length > options.limit, qualified: 0, pending: 0, heldIds: [] };
  const changedTransfers = new Map<string, Transfer>(), changedCompletions = new Map<string, CompletedTransfer>();
  const changedSettlements = new Map<string, ReceiptSettlement>();
  let examining: string | undefined;
  try {
    const checkpoint = await qualifyFinality(clients, await loadCheckpoint(store, clients));
    const authenticate = createReceiptAuthenticator(clients, checkpoint);
    const references = new Map<string, bigint>();
    for (const transfer of page) {
      examining = transfer.id;
      if ((transfer.status === Status.Completed || transfer.status === Status.Refunded) && !transfer.completionAuth) {
        throw new Error('HOLD: terminal projection lacks authentication; migrate before repair');
      }
      if (options.direction === 'vara-to-eth') {
        const result = await reconcileOutbound(store, transfer, queue, manager, checkpoint, authenticate, references);
        if (result) {
          changedTransfers.set(transfer.id, result.transfer);
          changedCompletions.set(result.completion.id, result.completion);
          report.qualified++;
        } else {
          report.pending++;
          if (transfer.legacyResolution || transfer.completedAtTxHash) report.heldIds.push(transfer.id);
        }
      } else {
        if (transfer.completionAuth && transfer.completionAuth.kind !== 'gear-settlement') {
          throw new Error('HOLD: original inbound completion has another evidence kind');
        }
        const binding = transfer.completionAuth;
        const source = await authenticateSourceReceipt(clients, transfer, manager, checkpoint, authenticate);
        if (transfer.sourceReceiptAuth) referenceCheckpoint(references, transfer.sourceReceiptAuth.checkpointId, transfer.blockNumber);
        if (transfer.completionAuth) referenceCheckpoint(references, transfer.completionAuth.sourceCheckpointId, transfer.blockNumber);
        const ids = new Set<string>();
        if (transfer.sourceTransactionIndex != null && transfer.sourceLogIndex != null) {
          const relays = await store.find(ReceiptRelay, { where: { blockNumber: transfer.blockNumber } });
          for (const relay of relays) {
            if (relay.id.split(':')[1] === String(transfer.sourceTransactionIndex)) ids.add(`${relay.id}:${transfer.sourceLogIndex}`);
          }
        }
        if (transfer.completionAuth) ids.add(transfer.completionAuth.receiptSettlementId);
        const settlements = ids.size ? await store.find(ReceiptSettlement, { where: { id: In([...ids]) } }) : [];
        if (binding && !settlements.some((s) => s.id === binding.receiptSettlementId)) {
          throw new Error('HOLD: missing original qualified receipt settlement');
        }
        const updates = await reconcileReceiptSettlements(store, settlements, async (candidate) => {
          if (candidate.id !== transfer.id) throw new Error('HOLD: receipt settlement points outside the selected original');
          return source;
        });
        const repaired = updates.transfers.find((row) => row.id === transfer.id);
        if (!repaired) {
          if (transfer.completionAuth) throw new Error('HOLD: original qualified inbound outcome no longer matches');
          report.pending++;
          if (!source) report.heldIds.push(transfer.id);
          continue;
        }
        if (!repaired.sourceReceiptAuth || !repaired.completionAuth || repaired.completionAuth.kind !== 'gear-settlement') {
          throw new Error('HOLD: settlement lacks qualified original source evidence');
        }
        if (transfer.completionAuth && (!isDeepStrictEqual(transfer.completionAuth, repaired.completionAuth) ||
            !isDeepStrictEqual(transfer.sourceReceiptAuth, repaired.sourceReceiptAuth) || transfer.status !== repaired.status ||
            !sameCoordinates(transfer, repaired.completedAt!, BigInt(repaired.completedAtBlock!), repaired.completedAtTxHash!.toLowerCase()))) {
          throw new Error('HOLD: original qualified inbound resolution changed');
        }
        for (const row of updates.transfers) changedTransfers.set(row.id, row);
        for (const row of updates.settlements) changedSettlements.set(row.id, row);
        report.qualified++;
      }
    }
    examining = undefined;
    // Existing authorization references must resolve; do not repair them by rebinding
    // to today's checkpoint. The fixed page checkpoint itself is inserted below.
    if (references.size) {
      const retained = await store.find(EthereumFinalityCheckpoint, { where: { id: In([...references.keys()]) } });
      for (const [id, minimumHeight] of references) {
        const row = retained.find((record) => record.id === id);
        if (!row || checkpointId(row.checkpoint) !== id || BigInt(row.blockNumber) !== BigInt(row.checkpoint.number) ||
            BigInt(row.checkpoint.number) < minimumHeight) {
          throw new Error('HOLD: missing or contradictory original authorization checkpoint');
        }
        await qualifyFinality(clients, row.checkpoint);
      }
    }
    await qualifyFinality(clients, checkpoint);
    if (options.apply) {
      await retainCheckpoint(store, checkpoint);
      if (changedCompletions.size) await store.save([...changedCompletions.values()]);
      if (changedSettlements.size) await store.save([...changedSettlements.values()]);
      if (changedTransfers.size) await store.save([...changedTransfers.values()]);
    }
    return report;
  } catch (error) {
    const failure = new Error(error instanceof Error ? error.message : String(error), { cause: error });
    Object.assign(failure, { heldIds: examining ? [examining] : page.map((row) => row.id) });
    throw failure;
  }
}

async function reconcileOutbound(
  store: Store, transfer: Transfer, queue: string, manager: string, checkpoint: AgreedCheckpoint,
  authenticate: ReceiptAuthenticator, references: Map<string, bigint>,
): Promise<{ transfer: Transfer; completion: CompletedTransfer } | undefined> {
  if (transfer.completionAuth && transfer.completionAuth.kind !== 'ethereum-receipt') {
    throw new Error('HOLD: original outbound completion has another evidence kind');
  }
  const id = `outbound:${queue}:${transfer.nonce}`;
  if (transfer.completionAuth && transfer.completionAuth.completionId !== id) throw new Error('HOLD: original completion ID changed');
  const saved = await store.findOneBy(CompletedTransfer, { id });
  if (transfer.completionAuth && (!saved || !saved.receiptAuth)) throw new Error('HOLD: missing original qualified completion');
  if (saved && !saved.outboundReceipt) throw new Error('HOLD: missing retained original completion receipt evidence');
  if (saved?.receiptAuth) referenceCheckpoint(references, saved.receiptAuth.checkpointId, saved.blockNumber);
  const legacy = transfer.legacyResolution;
  const originalHash = legacy?.completedAtTxHash ?? transfer.completedAtTxHash;
  const originalBlock = legacy?.completedAtBlock ?? transfer.completedAtBlock;
  if (saved && (saved.srcNetwork !== Network.Vara || saved.destNetwork !== Network.Ethereum ||
      (originalHash && saved.txHash.toLowerCase() !== originalHash.toLowerCase()) ||
      (originalBlock != null && BigInt(saved.blockNumber) !== BigInt(originalBlock)))) {
    throw new Error('HOLD: retained completion conflicts with original coordinates');
  }
  const txHash = saved?.txHash ?? originalHash, blockNumber = saved?.blockNumber ?? originalBlock;
  if (!txHash || blockNumber == null) return undefined;
  if (txHash.trim() !== txHash || !/^0x[0-9a-fA-F]{64}$/.test(txHash)) throw new Error('HOLD: malformed original completion hash');
  const evidence = saved?.outboundReceipt;
  if (evidence && (evidence.nonce !== transfer.nonce || evidence.queue.toLowerCase() !== queue || evidence.manager.toLowerCase() !== manager)) {
    throw new Error('HOLD: retained completion routing changed');
  }
  const expectedHash = saved?.receiptAuth?.receipt.blockHash ?? evidence?.receipt.blockHash;
  const original = await authenticate(txHash, { height: height(blockNumber), ...(expectedHash ? { hash: expectedHash } : {}) });
  if (!original) {
    if (saved || transfer.completionAuth) throw new Error('HOLD: original retained completion receipt unavailable');
    return undefined;
  }
  if (evidence) assertRetainedReceipt(evidence.receipt, original.receipt);
  if (saved?.receiptAuth && (saved.receiptAuth.version !== 1 ||
      saved.receiptAuth.blockTimestamp !== original.blockTimestamp || !isDeepStrictEqual(saved.receiptAuth.receipt, original.receipt))) {
    throw new Error('HOLD: original qualified completion receipt changed');
  }
  const effect = qualifyOutboundCompletion(transfer, original, queue, manager);
  if (!effect) {
    if (transfer.completionAuth || (saved?.receiptAuth && saved.matched)) throw new Error('HOLD: original qualified completion effect no longer matches');
    return undefined;
  }
  if (evidence && evidence.logIndex !== effect.processedLogIndex) throw new Error('HOLD: original completion log changed');
  const timestamp = receiptDate(original.blockTimestamp), block = BigInt(original.receipt.blockNumber), hash = original.receipt.transactionHash;
  const receiptAuth = saved?.receiptAuth ?? { version: 1 as const, checkpointId: checkpointId(checkpoint),
    receipt: original.receipt, blockTimestamp: original.blockTimestamp };
  const completionAuth = { version: 1 as const, kind: 'ethereum-receipt' as const,
    checkpointId: receiptAuth.checkpointId, completionId: id, processedLogIndex: effect.processedLogIndex };
  if (transfer.completionAuth && (!isDeepStrictEqual(transfer.completionAuth, completionAuth) || transfer.status !== Status.Completed ||
      !sameCoordinates(transfer, timestamp, block, hash))) {
    throw new Error('HOLD: original qualified outbound resolution changed');
  }
  const completion = new CompletedTransfer({ ...(saved ?? { id, srcNetwork: Network.Vara, destNetwork: Network.Ethereum,
    txHash: hash, blockNumber: block, timestamp }),
    outboundReceipt: evidence ?? { receipt: original.receipt, logIndex: effect.processedLogIndex, nonce: transfer.nonce, queue, manager },
    receiptAuth, matched: true });
  return { completion, transfer: transfer.completionAuth ? transfer : new Transfer({ ...transfer, status: Status.Completed,
    completedAt: timestamp, completedAtBlock: block, completedAtTxHash: hash, completionAuth }) };
}

async function main(): Promise<void> {
  const options = parseReconcileArgs(process.argv.slice(2));
  // Reuse options, never initialize the migration-running singleton.
  const address = (name: string) => {
    const value = process.env[name];
    if (!value || value.trim() !== value || !/^0x[0-9a-fA-F]{40}$/.test(value)) throw new Error(`Invalid or missing ${name}`);
    return value.toLowerCase();
  };
  const queue = address('ETH_MSQ_QUEUE'), manager = address('ETH_ERC20_MANAGER');
  const clients = createEthereumEvidenceClients();
  const database = new DataSource({ ...existing.options, migrationsRun: false, synchronize: false, dropSchema: false, logging: false });
  try {
    await database.initialize();
    const runner = database.createQueryRunner();
    try {
      await runner.connect();
      await runner.startTransaction('SERIALIZABLE');
      if (!options.apply) await runner.query('SET TRANSACTION READ ONLY');
      const report = await reconcilePage(new Store(() => runner.manager), clients, options, queue, manager);
      if (options.apply) await runner.commitTransaction();
      else await runner.rollbackTransaction();
      console.log(JSON.stringify(report));
    } catch (error) {
      if (runner.isTransactionActive) await runner.rollbackTransaction();
      throw error;
    } finally {
      await runner.release();
    }
  } finally {
    try {
      if (database.isInitialized) await database.destroy();
    } finally {
      destroyEthereumEvidenceClients();
    }
  }
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  main().catch((error: unknown) => {
    console.error(JSON.stringify({ error: error instanceof Error ? error.message : String(error),
      heldIds: error && typeof error === 'object' && 'heldIds' in error ? error.heldIds : [] }));
    process.exitCode = 1;
  });
}
