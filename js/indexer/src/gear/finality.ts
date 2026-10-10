import { setTimeout } from 'node:timers/promises';
import type { ProcessorContext } from './processor.js';

export async function requireFinalizedBatch(ctx: ProcessorContext): Promise<void> {
  if (!ctx.blocks.length) return;
  const rpc = ctx._chain.rpc;
  const options = { timeout: 15_000, retryAttempts: 2 };
  const lastHeight = ctx.blocks.reduce((height, block) => Math.max(height, block.header.height), 0);
  let finalizedHash = '';
  let finalizedHeight = '';
  for (let attempt = 0; attempt < 5; attempt++) {
    finalizedHash = await rpc.call<string>('chain_getFinalizedHead', [], options);
    if (!/^0x[0-9a-f]{64}$/i.test(finalizedHash)) throw new Error('HOLD: finalized Gear hash unavailable');
    const header = await rpc.call<{ number: string } | null>('chain_getHeader', [finalizedHash], options);
    if (!header || !/^0x[0-9a-f]+$/i.test(header.number)) throw new Error('HOLD: finalized Gear header unavailable');
    finalizedHeight = header.number;
    if (BigInt(finalizedHeight) >= BigInt(lastHeight)) break;
    if (attempt === 4) throw new Error('HOLD: batch exceeds canonical finalized Gear height');
    await setTimeout(30_000);
  }
  for (let offset = 0; offset < ctx.blocks.length; offset += 64) {
    const blocks = ctx.blocks.slice(offset, offset + 64);
    const hashes = await rpc.batchCall<string>(blocks.map(({ header }) => ({
      method: 'chain_getBlockHash', params: [header.height],
    })), options);
    if (hashes.length !== blocks.length || hashes.some((hash, i) => hash?.toLowerCase() !== blocks[i].header.hash.toLowerCase())) {
      throw new Error('HOLD: noncanonical Gear batch');
    }
  }
  const anchor = await rpc.call<string>('chain_getBlockHash', [finalizedHeight], options);
  if (anchor?.toLowerCase() !== finalizedHash.toLowerCase()) throw new Error('HOLD: finalized Gear anchor changed');
}
