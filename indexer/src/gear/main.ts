import { TypeormDatabase } from '@subsquid/typeorm-store';
import { randomUUID } from 'crypto';
import {
  BridgingPaidEvent,
  BridgingRequested,
  HistoricalProxyAddressChanged,
  Relayed,
  TokenMappingAdded,
  TokenMappingRemoved,
} from './types';
import { ethNonce, gearNonce } from '../common';
import { ProcessorContext, getProcessor } from './processor';
import { Network, Status, Transfer } from '../model';
import {
  BridgingPaymentMethods,
  BridgingPaymentServices,
  HistoricalProxyMethods,
  HistoricalProxyServices,
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
        const name = programs.get(msg.source)!;
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
                const id = randomUUID();

                const transfer = new Transfer({
                  id,
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
            state.setCompletedTransfer(nonce, timestamp);
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
    [ProgramName.HistoricalProxy]: config.hisotricalProxy,
    [ProgramName.BridgingPayment]: config.bridgingPayment,
  })) as Map<string, ProgramName>;

  const processor = getProcessor(Array.from(programs.keys()));

  processor.run(db, handler);
};

runProcessor().catch((error) => {
  console.error(error);
  process.exit(1);
});
