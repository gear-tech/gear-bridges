import { Relayed, UserMessageSentHandlerContext } from '../types/index.js';
import { HistoricalProxyEvents, HistoricalProxyServices } from '../util.js';

export function handleHistoricalProxyEvents(ctx: UserMessageSentHandlerContext) {
  const { service, method } = ctx;
  if (service !== HistoricalProxyServices.HistoricalProxy) return;
  if (method !== HistoricalProxyEvents.Relayed) return;

  const relayed = ctx.decoder.decodeEvent<Relayed>(service, method, ctx.event.args.message.payload);
  // The proof envelope is not the consumer's economic result. Record only its
  // authenticated slot -> execution block link; settlement comes from the manager.
  ctx.state.recordReceiptRelay(relayed);
}
