import { ProgramName, VftManagerMethods, VftManagerServices } from '../util.js';
import { InitiatedTransfer, Network, Status, Transfer } from '../../model/index.js';
import { setPrograms, updateId } from '../programIds.js';
import { gearNonce } from '../../common/index.js';
import {
  BridgingRequested,
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
    case VftManagerMethods.BridgingRequested: {
      // TODO: queue_id
      const { nonce, vara_token_id, sender, receiver, amount } = decoder.decodeEvent<BridgingRequested>(
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
      ctx.log.info(`Historical proxy program changed to ${data.new}`);
      await updateId(ProgramName.HistoricalProxy, data.new);
      await state.save();
      await setPrograms();
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

export function handleVftManagerInMsg(ctx: MessageQueuedContext) {
  const { payload } = ctx.event.call!.args;

  const service = ctx.decoder.service(payload);
  if (service !== VftManagerServices.VftManager) return;

  const method = ctx.decoder.method(payload);
  if (method !== VftManagerMethods.RequestBridging) return;

  const transfer = new InitiatedTransfer({
    id: ctx.event.args.id,
    txHash: ctx.event.extrinsic!.hash,
    blockNumber: BigInt(ctx.blockHeader.height),
  });

  return ctx.state.addInitiatedTransfer(transfer);
}
