import { TypeormDatabase } from '@subsquid/typeorm-store';

import { GearEthBridgeMessage, InitiatedTransfer, Network, Status, Transfer } from '../model/index.js';
import { ProcessorContext, getProcessor } from './processor.js';
import { ethNonce, gearNonce } from '../common/index.js';
import { getDecoder, initDecoders } from './decoders.js';
import { getProgramInheritor } from './rpc-queries.js';
import { init, updateId } from './programIds.js';
import { BatchState } from './batch-state.js';
import { config } from './config.js';
import {
  BridgingPaymentMethods,
  BridgingPaymentServices,
  CheckpointClientMethods,
  CheckpointClientServices,
  HistoricalProxyMethods,
  HistoricalProxyServices,
  isEthBridgeMessageQueued,
  isMessageQueued,
  isProgramChanged,
  isUserMessageSent,
  ProgramName,
  VftManagerMethods,
  VftManagerServices,
} from './util.js';
import {
  BridgingPaidEvent,
  BridgingRequested,
  HistoricalProxyAddressChanged,
  NewCheckpointEvent,
  Relayed,
  TokenMappingAdded,
  TokenMappingRemoved,
} from './types';

const state = new BatchState();

let programs: Map<string, ProgramName>;

async function setPrograms() {
  programs = await init({
    [ProgramName.VftManager]: config.vftManager,
    [ProgramName.HistoricalProxy]: config.historicalProxy,
    [ProgramName.BridgingPayment]: config.bridgingPayment,
    [ProgramName.CheckpointClient]: config.checkpointClient,
  });
}

const handler = async (ctx: ProcessorContext) => {
  await state.new(ctx);

  for (const block of ctx.blocks) {
    const timestamp = new Date(block.header.timestamp!);
    const blockNumber = BigInt(block.header.height);

    for (const event of block.events) {
      if (isProgramChanged(event)) {
        const { id, change } = event.args;

        if (change.__kind == 'Inactive') {
          if (programs.has(id)) {
            ctx.log.info(`Program ${programs.get(id)} (${id}) exited.`);
            const inheritor = await getProgramInheritor(ctx._chain.rpc, block.header._runtime, id, block.header.hash);
            ctx.log.info(`Program inheritor ${inheritor}`);
            await updateId(programs.get(id)!, inheritor);
            ctx.log.info(`Program id updated from ${id} to ${inheritor}`);
            await setPrograms();
          } else {
            const vftTokens = state.getActiveVaraTokens();

            if (vftTokens.includes(id.toLowerCase())) {
              await state.upgradePair(id, block.header);
            }
          }
        }
        continue;
      }
      if (isUserMessageSent(event)) {
        if (!programs.has(event.args.message.source)) continue;

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
                  nonce: gearNonce(BigInt(nonce)),
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
                await setPrograms();
                continue;
              }
              case VftManagerMethods.RequestBridging: {
                const data = decoder.decodeOutput<{ ok: [nonce: string] }>(service, method, msg.payload);
                if (data.ok) {
                  await state.handleRequestBridgingReply(msg.details.to, gearNonce(BigInt(data.ok[0])));
                }
                continue;
              }
              default: {
                continue;
              }
            }
            break;
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

            state.updateTransferStatus(gearNonce(BigInt(nonce)), Status.Bridging);
            break;
          }
          case ProgramName.CheckpointClient: {
            if (service !== CheckpointClientServices.ServiceSyncUpdate) continue;
            if (method !== CheckpointClientMethods.NewCheckpoint) continue;

            const { slot, tree_hash_root } = decoder.decodeEvent<NewCheckpointEvent>(service, method, msg.payload);

            state.newSlot(BigInt(slot), tree_hash_root);
            continue;
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

        if (!event.call) {
          ctx.log.error({ event }, 'Event call is undefined');
          continue;
        }

        const { payload } = event.call.args;

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
            nonce: gearNonce(BigInt(nonce)),
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

  await setPrograms();
  const processor = getProcessor();

  processor.run(db, handler);
};

runProcessor().catch((error) => {
  console.error(error);
  process.exit(1);
});
