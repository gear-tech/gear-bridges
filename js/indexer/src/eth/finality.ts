import { setTimeout } from 'node:timers/promises';
import type { Context } from './processor.js';

export async function requireFinalizedBatch(ctx: Context): Promise<void> {
  if (!ctx.blocks.length) return;
  const rpc = ctx._chain.client;
  const lastHeight = ctx.blocks.reduce((height, block) => Math.max(height, block.header.height), 0);
  let finalized: { number: string; hash: string } | null = null;
  // A scheduling buffer is not finality. HOLD the entire batch, never filter its tail.
  for (let attempt = 0; attempt < 5; attempt++) {
    finalized = await rpc.call('eth_getBlockByNumber', ['finalized', false]);
    if (!finalized || !/^0x[0-9a-f]+$/i.test(finalized.number) || !/^0x[0-9a-f]{64}$/i.test(finalized.hash)) {
      throw new Error('HOLD: finalized Ethereum header unavailable or malformed');
    }
    if (BigInt(finalized.number) >= BigInt(lastHeight)) break;
    if (attempt === 4) throw new Error('HOLD: batch exceeds canonical finalized Ethereum height');
    await setTimeout(30_000);
  }
  for (let offset = 0; offset < ctx.blocks.length; offset += 64) {
    const blocks = ctx.blocks.slice(offset, offset + 64);
    const headers = await rpc.batchCall(blocks.map(({ header }) => ({
      method: 'eth_getBlockByNumber', params: [`0x${header.height.toString(16)}`, false],
    })));
    if (headers.length !== blocks.length) throw new Error('HOLD: incomplete canonical Ethereum headers');
    for (let i = 0; i < blocks.length; i++) {
      const expected = blocks[i].header;
      const actual = headers[i];
      if (!actual || BigInt(actual.number) !== BigInt(expected.height) ||
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
