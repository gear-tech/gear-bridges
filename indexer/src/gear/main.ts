import { TypeormDatabase } from '@subsquid/typeorm-store';
import { randomUUID } from 'crypto';
import {
  BridgingPaidEvent,
  BridgingRequested,
  HistoricalProxyAddressChanged,
  Relayed,
  RequestBridgingArgs,
  TokenMappingAdded,
  TokenMappingRemoved,
} from './types';
import { ethNonce, gearNonce, gearNonceFromNumber, mapKeys } from '../common';
import { ProcessorContext, getProcessor } from './processor';
import { EthBridgeProgram, GearEthBridgeMessage, InitiatedTransfer, Network, Status, Transfer } from '../model';
import {
  BridgingPaymentMethods,
  BridgingPaymentServices,
  HistoricalProxyMethods,
  HistoricalProxyServices,
  isEthBridgeMessageQueued,
  isMessageQueued,
  isProgramChanged,
  isUserMessageSent,
  ProgramName,
  VftManagerMethods,
  VftManagerServices,
} from './util';
import { config } from './config';
import { init, updateId } from './programIds';
import { getProgramInheritor } from './rpc-queries';
import { BatchState } from './batch-state';
import { getDecoder, initDecoders } from './decoders';

const state = new BatchState();

let programs: Map<string, ProgramName>;

const handler = async (ctx: ProcessorContext) => {
  await state.new(ctx);

  for (const block of ctx.blocks) {
    const timestamp = new Date(block.header.timestamp!);
    const blockNumber = BigInt(block.header.height);

    for (const event of block.events) {
      if (isProgramChanged(event)) {
        const { id, change } = event.args;

        if (change.__kind == 'Inactive') {
          const vftTokens = state.getActiveVaraTokens();

          if (programs.has(id)) {
            const inheritor = await getProgramInheritor(ctx._chain.rpc, block.header._runtime, id, block.header.hash);
            await updateId(programs.get(id)!, inheritor);
            await state.save();
            process.exit(0);
          }

          if (vftTokens.includes(id.toLowerCase())) {
            await state.upgradePair(id, block.header);
          }
        }
        continue;
      }
      if (isUserMessageSent(event)) {
        const msg = event.args.message;
        const name = programs.get(msg.source);
        if (!name) {
          ctx.log.error(`Failed to get program name and decoder for ${msg.source}`);
          continue;
        }

        const decoder = getDecoder(name);
        const service = decoder.service(msg.payload);
        const method = decoder.method(msg.payload);

        switch (name) {
          case ProgramName.VftManager: {
            if (service !== VftManagerServices.VftManager) continue;

            switch (method) {
              case VftManagerMethods.BridgingRequested: {
                const { nonce, vara_token_id, sender, receiver, amount } = decoder.decodeEvent<BridgingRequested>(
                  service,
                  method,
                  msg.payload,
                );

                const transfer = new Transfer({
                  id: msg.id,
                  txHash: event.extrinsic!.hash,
                  blockNumber: blockNumber,
                  timestamp,
                  nonce: gearNonce(nonce),
                  sourceNetwork: Network.Vara,
                  source: vara_token_id,
                  destNetwork: Network.Ethereum,
                  status: Status.AwaitingPayment,
                  sender,
                  receiver,
                  amount: BigInt(amount),
                });
                await state.addTransfer(transfer);
                break;
              }
              case VftManagerMethods.TokenMappingAdded: {
                const { vara_token_id, eth_token_id, supply_type } = decoder.decodeEvent<TokenMappingAdded>(
                  service,
                  method,
                  msg.payload,
                );

                await state.addPair(
                  vara_token_id.toLowerCase(),
                  eth_token_id.toLowerCase(),
                  supply_type === 'Ethereum' ? Network.Ethereum : Network.Vara,
                  block.header,
                );
                break;
              }
              case VftManagerMethods.TokenMappingRemoved: {
                const { vara_token_id, eth_token_id } = decoder.decodeEvent<TokenMappingRemoved>(
                  service,
                  method,
                  msg.payload,
                );
                state.removePair(vara_token_id, eth_token_id, blockNumber);
                break;
              }
              case VftManagerMethods.HistoricalProxyAddressChanged: {
                const data = decoder.decodeEvent<HistoricalProxyAddressChanged>(service, method, msg.payload);
                ctx.log.info(`Historical proxy program changed to ${data.new}`);
                await updateId(ProgramName.HistoricalProxy, data.new);
                await state.save();
                process.exit(0);
              }
              case VftManagerMethods.RequestBridging: {
                const data = decoder.decodeOutput<{ ok: [nonce: string] }>(service, method, msg.payload);
                if (data.ok) {
                  await state.handleRequestBridgingReply(msg.details.to, gearNonce(data.ok[0]));
                }
                continue;
              }
              default: {
                continue;
              }
            }
          }
          case ProgramName.HistoricalProxy: {
            if (service !== HistoricalProxyServices.HistoricalProxy) continue;
            if (method !== HistoricalProxyMethods.Relayed) continue;

            const { block_number, transaction_index } = decoder.decodeEvent<Relayed>(service, method, msg.payload);

            const nonce = ethNonce(`${block_number}${transaction_index}`);
            state.setCompletedTransfer(nonce, timestamp, blockNumber, event.extrinsic!.hash);
            break;
          }
          case ProgramName.BridgingPayment: {
            if (service !== BridgingPaymentServices.BridgingPayment) continue;
            if (method !== BridgingPaymentMethods.BridgingPaid) continue;

            const { nonce } = decoder.decodeEvent<BridgingPaidEvent>(service, method, msg.payload);

            state.updateTransferStatus(gearNonce(nonce), Status.Bridging);
            break;
          }
        }
      }

      if (isMessageQueued(event)) {
        const { id, destination } = event.args;
        const name = programs.get(destination);

        if (!name) continue;

        if (name !== ProgramName.VftManager) continue;
        const decoder = getDecoder(name);

        if (event.call!.name !== `Gear.send_message`) continue;

        const { payload } = event.call?.args;

        const service = decoder.service(payload);
        if (service !== VftManagerServices.VftManager) continue;

        const method = decoder.method(payload);
        if (method !== VftManagerMethods.RequestBridging) continue;

        const transfer = new InitiatedTransfer({
          id,
          txHash: event.extrinsic!.hash,
          blockNumber: blockNumber,
        });
        await state.addInitiatedTransfer(transfer);
        continue;
      }

      if (isEthBridgeMessageQueued(event)) {
        const {
          message: { nonce },
          hash,
        } = event.args;

        state.addEthBridgeMessage(
          new GearEthBridgeMessage({
            id: hash,
            nonce: gearNonceFromNumber(nonce),
            blockNumber,
          }),
        );
      }
    }
  }

  await state.save();
};

const runProcessor = async () => {
  await initDecoders();

  const db = new TypeormDatabase({
    supportHotBlocks: true,
    stateSchema: 'gear_processor',
  });

  programs = (await init({
    [ProgramName.VftManager]: config.vftManager,
    [ProgramName.HistoricalProxy]: config.historicalProxy,
    [ProgramName.BridgingPayment]: config.bridgingPayment,
  })) as Map<string, ProgramName>;

  const processor = getProcessor(mapKeys(programs));

  processor.run(db, handler);
};

runProcessor().catch((error) => {
  console.error(error);
  process.exit(1);
});
