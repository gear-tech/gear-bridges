import { ethNonce } from '../../common/index.js';
import { Relayed, UserMessageSentHandlerContext } from '../types/index.js';
import { HistoricalProxyEvents, HistoricalProxyServices } from '../util.js';

export function handleHistoricalProxyEvents(ctx: UserMessageSentHandlerContext) {
  const { service, method } = ctx;
  if (service !== HistoricalProxyServices.HistoricalProxy) return;
  if (method !== HistoricalProxyEvents.Relayed) return;

  const { block_number, transaction_index } = ctx.decoder.decodeEvent<Relayed>(
    service,
    method,
    ctx.event.args.message.payload,
  );

  const nonce = ethNonce(`${block_number}${transaction_index}`);
  ctx.state.setCompletedTransfer(
    nonce,
    new Date(ctx.blockHeader.timestamp!),
    BigInt(ctx.blockHeader.height),
    ctx.event.extrinsic!.hash,
  );
}
