import { BridgingPaidEvent, PriorityBridgingPaid, UserMessageSentHandlerContext } from '../types/index.js';
import { BridgingPaymentMethods, BridgingPaymentServices } from '../util.js';
import { gearNonce } from '../../common/index.js';
import { Status } from '../../model/index.js';

export function handleBridgingPaymentEvents(ctx: UserMessageSentHandlerContext) {
  if (ctx.service !== BridgingPaymentServices.BridgingPayment) return;

  const isPriority = ctx.method === BridgingPaymentMethods.PriorityBridgingPaid;

  switch (ctx.method) {
    case BridgingPaymentMethods.BridgingPaid:
    case BridgingPaymentMethods.PriorityBridgingPaid: {
      const { nonce } = ctx.decoder.decodeEvent<BridgingPaidEvent | PriorityBridgingPaid>(
        ctx.service,
        ctx.method,
        ctx.event.args.message.payload,
      );

      ctx.state.updateTransferStatus(gearNonce(BigInt(nonce)), Status.Bridging, isPriority);
    }
  }
}
