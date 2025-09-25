import { programs, setPrograms, updateId } from '../programIds.js';
import { ProgramChangedHandlerContext } from '../types/index.js';
import { getProgramInheritor } from '../rpc-queries.js';

export async function handleProgramChangedEvent(ctx: ProgramChangedHandlerContext) {
  const { id, change } = ctx.event.args;

  if (change.__kind == 'Inactive') {
    if (programs.has(id)) {
      ctx.log.info(`Program ${programs.get(id)} (${id}) exited.`);
      const inheritor = await getProgramInheritor(ctx.rpc, ctx.blockHeader._runtime, id, ctx.blockHeader.hash);
      ctx.log.info(`Program inheritor ${inheritor}`);
      await updateId(programs.get(id)!, inheritor);
      ctx.log.info(`Program id updated from ${id} to ${inheritor}`);
      await setPrograms();
    } else {
      const vftTokens = ctx.state.getActiveVaraTokens();

      if (vftTokens.includes(id.toLowerCase())) {
        await ctx.state.upgradePair(id, ctx.blockHeader);
      }
    }
  }
}
