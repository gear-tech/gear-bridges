import { programs, setPrograms, updateId } from '../programIds.js';
import { ProgramChangedHandlerContext } from '../types/index.js';
import { getProgramInheritor } from '../rpc-queries.js';

export async function handleProgramChangedEvent(ctx: ProgramChangedHandlerContext) {
  const { id, change } = ctx.event.args;

  if (change.__kind == 'Inactive') {
    if (programs.has(id)) {
      ctx.log.info({ programName: programs.get(id), programId: id }, 'Program exited');
      const inheritor = await getProgramInheritor(ctx.rpc, ctx.blockHeader._runtime, id, ctx.blockHeader.hash);
      ctx.log.info({ programId: id, inheritorId: inheritor }, 'Program inheritor found');
      await updateId(programs.get(id)!, inheritor);
      ctx.log.info({ oldProgramId: id, newProgramId: inheritor }, 'Program ID updated');
      await setPrograms();
    } else {
      const vftTokens = ctx.state.getActiveVaraTokens();

      if (vftTokens.includes(id.toLowerCase())) {
        await ctx.state.upgradePair(id, ctx.blockHeader);
      }
    }
  }
}
