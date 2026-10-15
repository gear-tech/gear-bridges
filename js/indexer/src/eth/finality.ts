import { setTimeout } from 'node:timers/promises';
import type { Context } from './processor.js';

export async function requireFinalizedBatch(
  ctx: Context,
  headers: readonly { height: number; hash: string }[] = ctx.blocks.map(({ header }) => header),
): Promise<void> {
  if (!headers.length) return;
  const rpc = ctx._chain.client;
  const lastHeight = headers.reduce((height, header) => Math.max(height, header.height), 0);
  let finalized: { number: string; hash: string } | null = null;
  // A scheduling buffer is not finality. HOLD the entire batch, never filter its tail.
  while (true) {
    const current = await rpc.call<{ number: string; hash: string } | null>('eth_getBlockByNumber', ['finalized', false]);
    if (!current || typeof current.number !== 'string' || !/^0x[0-9a-f]+$/i.test(current.number) ||
        typeof current.hash !== 'string' || !/^0x[0-9a-f]{64}$/i.test(current.hash)) {
      throw new Error('HOLD: finalized Ethereum header unavailable or malformed');
    }
    if (finalized) {
      if (BigInt(current.number) < BigInt(finalized.number)) {
        throw new Error('HOLD: finalized Ethereum height regressed');
      }
      const previous = await rpc.call<{ number: string; hash: string } | null>('eth_getBlockByNumber', [finalized.number, false]);
      if (!previous || typeof previous.number !== 'string' || !/^0x[0-9a-f]+$/i.test(previous.number) ||
          BigInt(previous.number) !== BigInt(finalized.number) ||
          previous.hash?.toLowerCase() !== finalized.hash.toLowerCase() ||
          (BigInt(current.number) === BigInt(finalized.number) && current.hash.toLowerCase() !== finalized.hash.toLowerCase())) {
        throw new Error('HOLD: previous finalized Ethereum anchor changed');
      }
    }
    finalized = current;
    if (BigInt(finalized.number) >= BigInt(lastHeight)) break;
    await setTimeout(30_000);
  }
  for (let offset = 0; offset < headers.length; offset += 64) {
    const blocks = headers.slice(offset, offset + 64);
    const canonical = await rpc.batchCall(blocks.map((header) => ({
      method: 'eth_getBlockByNumber', params: [`0x${header.height.toString(16)}`, false],
    })));
    if (canonical.length !== blocks.length) throw new Error('HOLD: incomplete canonical Ethereum headers');
    for (let i = 0; i < blocks.length; i++) {
      const expected = blocks[i];
      const actual = canonical[i];
      if (!actual || typeof actual.number !== 'string' || !/^0x[0-9a-f]+$/i.test(actual.number) ||
          BigInt(actual.number) !== BigInt(expected.height) ||
          actual.hash?.toLowerCase() !== expected.hash.toLowerCase()) {
        throw new Error(`HOLD: noncanonical Ethereum block ${expected.height}`);
      }
    }
  }
  const anchor = await rpc.call<{ hash: string } | null>('eth_getBlockByNumber', [finalized!.number, false]);
  if (anchor?.hash?.toLowerCase() !== finalized!.hash.toLowerCase()) {
    throw new Error('HOLD: canonical finalized Ethereum anchor changed');
  }
}
