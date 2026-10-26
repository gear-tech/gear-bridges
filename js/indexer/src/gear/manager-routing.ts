import { ZERO_ADDRESS } from 'sails-js';

import { GearManagerHandoff } from '../model/index.js';
import { config } from './config.js';
import { getDecoder } from './decoders.js';
import { Block, ProcessorContext } from './processor.js';
import { ProgramName } from './util.js';

interface CutoverStatus {
  phase: 'Active' | 'Draining' | 'AwaitingSource' | 'Transferring' | 'Retired' | 'Ready';
  predecessor: string | null;
  successor: string | null;
  minimum_receipt_slot: string | number;
  last_receipt_slot: string | number | null;
  config_fingerprint: string | null;
  transfer_count: number;
  completed_transfer_count: number;
  in_flight_transfer_count: number;
  generation: string | number;
  predecessor_generation: string | number | null;
  custody_started: boolean;
}

const actor = (id: unknown): id is string => typeof id === 'string' && /^0x[0-9a-f]{64}$/.test(id) && id !== ZERO_ADDRESS;
const slot = (value: string | number): bigint => {
  if ((typeof value === 'number' && !Number.isSafeInteger(value)) ||
      !/^(?:\d+|0x[0-9a-f]+)$/i.test(String(value))) throw new Error('HOLD: invalid cutover receipt slot');
  const result = BigInt(value);
  if (result > 0xffffffffffffffffn) throw new Error('HOLD: invalid cutover receipt slot');
  return result;
};

async function queryStatus(ctx: ProcessorContext, block: Block, id: string): Promise<CutoverStatus> {
  const decoder = getDecoder(ProgramName.VftManager);
  const payload = decoder.encodeQueryInput('VftManager', 'CutoverStatus', []);
  const reply = await ctx._chain.rpc.call('gear_calculateReplyForHandle', [ZERO_ADDRESS, id, payload, 1e10, 0, block.hash]);
  if (!reply?.code?.Success) throw new Error('HOLD: finalized manager CutoverStatus unavailable');
  const status = decoder.decodeQueryOutput<CutoverStatus>('VftManager', 'CutoverStatus', reply.payload);
  if (!['Active', 'Draining', 'AwaitingSource', 'Transferring', 'Retired', 'Ready'].includes(status.phase)) {
    throw new Error('HOLD: invalid manager cutover phase');
  }
  slot(status.minimum_receipt_slot);
  slot(status.generation);
  if (status.predecessor_generation !== null) slot(status.predecessor_generation);
  if (status.last_receipt_slot !== null) slot(status.last_receipt_slot);
  return status;
}

// The legacy role row is the trust anchor, not a mutable "latest manager" pointer.
// Links and projections share the processor transaction/cursor; replay uses only links
// already authenticated at that block, never a future successor from a prior run.
export async function authenticateManagerRouting(
  ctx: ProcessorContext,
  block: Block,
  root: string,
  history: GearManagerHandoff[],
): Promise<{ current: string; domains: Map<string, { minimum: bigint; last?: bigint }> }> {
  const activation = config.retainedCutoverFromBlock;
  if (history.some((row) => activation === undefined || BigInt(row.activationBlock) !== BigInt(activation))) {
    throw new Error('HOLD: retained manager capability boundary changed or removed');
  }
  if (history.length && !history.some((row) => row.id === root)) {
    throw new Error('HOLD: retained manager trust anchor changed');
  }
  const domains = new Map<string, { minimum: bigint; last?: bigint }>();
  let current = root;
  if (activation === undefined || block.height < activation) {
    domains.set(root, { minimum: 0n });
    return { current, domains };
  }
  let minimum: bigint | undefined;
  let predecessor: string | undefined;
  let pending: CutoverStatus | undefined;
  while (true) {
    if (domains.has(current)) throw new Error('HOLD: cyclic retained manager lineage');
    const recorded = history.find((row) => row.id === current);
    if (recorded && BigInt(recorded.blockNumber) <= BigInt(block.height)) {
      if (BigInt(recorded.blockNumber) === BigInt(block.height) && recorded.blockHash !== block.hash) {
        throw new Error('HOLD: retained manager authentication block changed');
      }
      if (minimum !== undefined && minimum !== slot(recorded.minimumReceiptSlot)) {
        throw new Error('HOLD: retained manager lineage floor changed');
      }
      domains.set(current, { minimum: slot(recorded.minimumReceiptSlot), last: slot(recorded.lastReceiptSlot) });
      minimum = slot(recorded.lastReceiptSlot) + 1n;
      predecessor = current;
      current = recorded.successor;
      continue;
    }
    const source = pending ?? await queryStatus(ctx, block, current);
    pending = undefined;
    if (minimum !== undefined && minimum !== slot(source.minimum_receipt_slot)) {
      throw new Error('HOLD: retained manager receipt floor changed');
    }
    if (predecessor !== undefined && (source.predecessor !== predecessor || source.phase === 'AwaitingSource')) {
      throw new Error('HOLD: authenticated manager readiness or predecessor changed');
    }
    domains.set(current, { minimum: slot(source.minimum_receipt_slot),
      last: source.last_receipt_slot === null ? undefined : slot(source.last_receipt_slot) });
    if (source.phase !== 'Retired') break;
    const successor = source.successor;
    if (!actor(successor) || domains.has(successor) || source.last_receipt_slot === null ||
        !/^0x[0-9a-f]{64}$/.test(source.config_fingerprint ?? '') ||
        slot(source.last_receipt_slot) < slot(source.minimum_receipt_slot) ||
        source.completed_transfer_count !== source.transfer_count || source.in_flight_transfer_count !== 0) {
      throw new Error('HOLD: invalid retired manager handoff');
    }
    const boundary = slot(source.last_receipt_slot);
    const destination = await queryStatus(ctx, block, successor);
    if (destination.predecessor !== current || slot(destination.minimum_receipt_slot) !== boundary + 1n ||
        destination.predecessor_generation === null || slot(destination.predecessor_generation) !== slot(source.generation) ||
        destination.config_fingerprint !== source.config_fingerprint) {
      throw new Error('HOLD: mismatched retained manager successor');
    }
    // Retirement and destination finalization are separate transactions. Keep
    // indexing the source while the authenticated successor is still preparing.
    if (destination.phase === 'AwaitingSource') break;
    if (!['Ready', 'Active', 'Draining', 'Transferring', 'Retired'].includes(destination.phase)) {
      throw new Error('HOLD: retained manager successor is not ready');
    }
    const handoff = new GearManagerHandoff({ id: current, successor,
      minimumReceiptSlot: slot(source.minimum_receipt_slot).toString(), lastReceiptSlot: boundary.toString(),
      configFingerprint: source.config_fingerprint!, blockNumber: BigInt(block.height), blockHash: block.hash,
      activationBlock: BigInt(activation) });
    if (recorded) {
      if (['successor', 'minimumReceiptSlot', 'lastReceiptSlot', 'configFingerprint'].some((key) =>
        String(recorded[key as keyof GearManagerHandoff]) !== String(handoff[key as keyof GearManagerHandoff]))) {
        throw new Error('HOLD: conflicting retained manager handoff');
      }
    } else {
      await ctx.store.save([handoff]);
      history.push(handoff);
    }
    minimum = boundary + 1n;
    predecessor = current;
    current = successor;
    pending = destination;
  }
  return { current, domains };
}
