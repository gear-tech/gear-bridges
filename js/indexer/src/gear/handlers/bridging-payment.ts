import { BridgingPaidEvent, PriorityBridgingPaid, UserMessageSentHandlerContext } from '../types/index.js';
import { BridgingPaymentMethods, BridgingPaymentServices } from '../util.js';
import { gearNonce } from '../../common/index.js';
import { Status } from '../../model/index.js';

export function handleBridgingPaymentEvents(ctx: UserMessageSentHandlerContext) {
  if (ctx.service !== BridgingPaymentServices.BridgingPayment) return;

  switch (ctx.method) {
    case BridgingPaymentMethods.BridgingPaid: {
      const { nonce } = ctx.decoder.decodeEvent<BridgingPaidEvent>(
        ctx.service,
        ctx.method,
        ctx.event.args.message.payload,
      );

      ctx.state.updateTransferStatus(gearNonce(BigInt(nonce)), Status.Bridging);
      break;
    }
    case BridgingPaymentMethods.PriorityBridgingPaid: {
      const { nonce } = ctx.decoder.decodeEvent<PriorityBridgingPaid>(
        ctx.service,
        ctx.method,
        ctx.event.args.message.payload,
      );

      ctx.state.setIsPriority(gearNonce(BigInt(nonce)));
      break;
    }
  }
}
