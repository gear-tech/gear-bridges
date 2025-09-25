import { NewCheckpointEvent, UserMessageSentHandlerContext } from '../types/index.js';
import { CheckpointClientMethods, CheckpointClientServices } from '../util.js';

export function handleCheckpointClientEvents(ctx: UserMessageSentHandlerContext) {
  const { service, method } = ctx;
  if (service !== CheckpointClientServices.ServiceSyncUpdate) return;
  if (method !== CheckpointClientMethods.NewCheckpoint) return;

  const { slot, tree_hash_root } = ctx.decoder.decodeEvent<NewCheckpointEvent>(
    service,
    method,
    ctx.event.args.message.payload,
  );

  ctx.state.newSlot(BigInt(slot), tree_hash_root);
}
