import { ProgramName, VftManagerMethods, VftManagerServices } from '../util.js';
import { InitiatedTransfer, Network, Status, Transfer } from '../../model/index.js';
import { gearNonce } from '../../common/index.js';
import {
  BridgingRequested,
  ReceiptDepositSettled,
  ReceiptDepositRefunded,
  HistoricalProxyAddressChanged,
  MessageQueuedContext,
  TokenMappingAdded,
  TokenMappingRemoved,
  UserMessageSentHandlerContext,
} from '../types/index.js';

export async function handleVftManagerEvents(ctx: UserMessageSentHandlerContext) {
  if (ctx.service !== VftManagerServices.VftManager) return;

  const { event, service, method, blockHeader, decoder, state } = ctx;
  const blockNumber = BigInt(blockHeader.height);
  const msg = event.args.message;

  switch (method) {
    case VftManagerMethods.ReceiptDepositSettled: {
      const deposit = decoder.decodeEvent<ReceiptDepositSettled>(service, method, msg.payload);
      requireReceiptDomain(ctx, deposit.slot);
      state.recordReceiptSettlement(deposit, new Date(blockHeader.timestamp!), blockNumber, event.extrinsic!.hash);
      return;
    }
    case VftManagerMethods.ReceiptDepositRefunded: {
      const deposit = decoder.decodeEvent<ReceiptDepositRefunded>(service, method, msg.payload);
      requireReceiptDomain(ctx, deposit.slot);
      state.recordReceiptRefund(deposit, new Date(blockHeader.timestamp!), blockNumber, event.extrinsic!.hash);
      return;
    }
    case VftManagerMethods.BridgingRequested: {
      const { nonce, vara_token_id, sender, receiver, amount, queue_id, hash } = decoder.decodeEvent<BridgingRequested>(
        service,
        method,
        msg.payload,
      );

      const transfer = new Transfer({
        id: msg.id,
        txHash: event.extrinsic!.hash,
        blockNumber,
        timestamp: new Date(blockHeader.timestamp!),
        nonce: gearNonce(BigInt(nonce)),
        sourceNetwork: Network.Vara,
        source: vara_token_id,
        destNetwork: Network.Ethereum,
        status: Status.AwaitingPayment,
        sender,
        receiver,
        amount: amount.toString(),
        ethBridgeBuiltInQueueId: BigInt(queue_id),
        ethBridgeBuiltInMsgHash: hash,
      });
      await state.addTransfer(transfer);
      return;
    }
    case VftManagerMethods.TokenMappingAdded: {
      const { vara_token_id, eth_token_id, supply_type } = ctx.decoder.decodeEvent<TokenMappingAdded>(
        ctx.service,
        ctx.method,
        msg.payload,
      );
      if (supply_type !== 'Ethereum' && supply_type !== 'Gear') throw new Error('HOLD: invalid token supply');

      await ctx.state.addPair(
        vara_token_id.toLowerCase(),
        eth_token_id.toLowerCase(),
        supply_type === 'Ethereum' ? Network.Ethereum : Network.Vara,
        ctx.blockHeader,
      );
      return;
    }
    case VftManagerMethods.TokenMappingRemoved: {
      const { vara_token_id, eth_token_id } = ctx.decoder.decodeEvent<TokenMappingRemoved>(
        service,
        method,
        msg.payload,
      );
      state.removePair(vara_token_id, eth_token_id, blockNumber);
      return;
    }
    case VftManagerMethods.HistoricalProxyAddressChanged: {
      const data = decoder.decodeEvent<HistoricalProxyAddressChanged>(service, method, msg.payload);
      ctx.log.info({ newProgramId: data.new }, 'Historical proxy program changed');
      await state.updateProgramId(ProgramName.HistoricalProxy, data.new);
      return;
    }
    case VftManagerMethods.RequestBridging: {
      const data = decoder.decodeOutput<{ ok: [nonce: string] }>(service, method, msg.payload);
      if (data.ok) {
        await state.handleRequestBridgingReply(msg.details.to, gearNonce(BigInt(data.ok[0])));
      }
      return;
    }
  }
}

function requireReceiptDomain(ctx: UserMessageSentHandlerContext, receiptSlot: string | number) {
  if (!ctx.receiptDomain) return;
  const slot = BigInt(receiptSlot);
  if (slot < ctx.receiptDomain.minimum || (ctx.receiptDomain.last !== undefined && slot > ctx.receiptDomain.last)) {
    throw new Error('HOLD: receipt event outside authenticated manager deployment domain');
  }
}

export function handleVftManagerInMsg(ctx: MessageQueuedContext) {
  const { payload } = ctx.event.call!.args;

  // Only untrusted input decoding is skippable. Projection/store failures below
  // must escape so the processor rolls back the complete callback.
  try {
    const service = ctx.decoder.service(payload);
    if (service !== VftManagerServices.VftManager) return;
    const method = ctx.decoder.method(payload);
    if (method !== VftManagerMethods.RequestBridging) return;
    ctx.decoder.decodeInput(service, method, payload);
  } catch (error) {
    ctx.log.warn({ block: ctx.blockHeader.height, blockHash: ctx.blockHeader.hash,
      event: ctx.event.id, message: ctx.event.args.id, txHash: ctx.event.extrinsic?.hash,
      error: error instanceof Error ? error.message : String(error) }, 'Skipping malformed queued manager request');
    return;
  }

  const transfer = new InitiatedTransfer({
    id: ctx.event.args.id,
    txHash: ctx.event.extrinsic!.hash,
    blockNumber: BigInt(ctx.blockHeader.height),
  });

  return ctx.state.addInitiatedTransfer(transfer);
}
