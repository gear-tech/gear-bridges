import { TypeormDatabase } from '@subsquid/typeorm-store';
import { randomUUID } from 'crypto';
import { BridgingPaidEvent, BridgingRequested, Relayed, TokenMappingAdded, TokenMappingRemoved } from './types';
import { ethNonce, gearNonce, TempState } from '../common';
import { ProcessorContext, getProcessor } from './processor';
import { Network, Status, Transfer } from '../model';
import { isProgramChanged, isUserMessageSent } from './util';
import { config } from './config';
import { Decoder } from './codec';
import { init, updateId } from './programIds';
import { getProgramInheritor, initDecoders } from './rpc-queries';

const tempState = new TempState(Network.Gear);

let vftManagerDecoder: Decoder;
let hisotricalProxyDecoder: Decoder;
let bridgingPaymentDecoder: Decoder;

const enum ProgramName {
  VftManager = 'vft_manager',
  HistoricalProxy = 'historical_proxy',
  BridgingPayment = 'bridging_payment',
}

let programs: Map<string, ProgramName>;

const handler = async (ctx: ProcessorContext) => {
  await tempState.new(ctx);

  const promises: Promise<void>[] = [];

  for (const block of ctx.blocks) {
    const timestamp = new Date(block.header.timestamp!);
    const blockNumber = block.header.height.toString();

    for (const event of block.events) {
      if (isProgramChanged(event)) {
        const { id, change } = event.args;

        if (change.__kind == 'Inactive') {
          if (programs.has(id)) {
            const inheritor = await getProgramInheritor(ctx._chain.rpc, block.header._runtime, id, block.header.hash);
            await updateId(programs.get(id)!, inheritor);
            await tempState.save();
            process.exit(0);
          }
        }
        continue;
      }
      if (isUserMessageSent(event)) {
        const msg = event.args.message;
        const name = programs.get(msg.source);
        switch (name) {
          case ProgramName.VftManager: {
            const service = vftManagerDecoder.service(msg.payload);
            if (service !== 'VftManager') continue;
            const method = vftManagerDecoder.method(msg.payload);

            switch (method) {
              case 'BridgingRequested': {
                const { nonce, vara_token_id, sender, receiver, amount } =
                  vftManagerDecoder.decodeEvent<BridgingRequested>(service, method, msg.payload);
                const id = randomUUID();

                const transfer = new Transfer({
                  id,
                  txHash: event.extrinsic!.hash,
                  blockNumber,
                  timestamp,
                  nonce: gearNonce(nonce),
                  sourceNetwork: Network.Gear,
                  source: vara_token_id,
                  destNetwork: Network.Ethereum,
                  status: Status.AwaitingPayment,
                  sender,
                  receiver,
                  amount: BigInt(amount),
                });
                promises.push(tempState.transferRequested(transfer));
                break;
              }
              case 'TokenMappingAdded': {
                const { vara_token_id, eth_token_id, supply_type } = vftManagerDecoder.decodeEvent<TokenMappingAdded>(
                  service,
                  method,
                  msg.payload,
                );

                promises.push(
                  tempState.addPair(
                    vara_token_id.toLowerCase(),
                    eth_token_id.toLowerCase(),
                    supply_type === 'Ethereum' ? Network.Ethereum : Network.Gear,
                    block.header,
                  ),
                );
                break;
              }
              case 'TokenMappingRemoved': {
                const { vara_token_id, eth_token_id } = vftManagerDecoder.decodeEvent<TokenMappingRemoved>(
                  service,
                  method,
                  msg.payload,
                );
                tempState.removePair(vara_token_id.toLowerCase(), eth_token_id.toLowerCase());
                break;
              }
              default: {
                continue;
              }
            }
          }
          case ProgramName.HistoricalProxy: {
            const service = hisotricalProxyDecoder.service(msg.payload);
            if (service !== 'HistoricalProxy') continue;
            const method = hisotricalProxyDecoder.method(msg.payload);
            if (method !== 'Relayed') continue;

            const { block_number, transaction_index } = hisotricalProxyDecoder.decodeEvent<Relayed>(
              service,
              method,
              msg.payload,
            );

            const nonce = ethNonce(`${block_number}${transaction_index}`);
            tempState.transferCompleted(nonce, timestamp);
            break;
          }
          case ProgramName.BridgingPayment: {
            const service = bridgingPaymentDecoder.service(msg.payload);
            if (service !== 'BridgingPayment') continue;
            const method = bridgingPaymentDecoder.method(msg.payload);
            if (method !== 'BridgingPaid') continue;

            const { nonce } = bridgingPaymentDecoder.decodeEvent<BridgingPaidEvent>(service, method, msg.payload);

            tempState.transferStatus(gearNonce(nonce), Status.Bridging);
            break;
          }
        }
      }
    }
  }

  await Promise.all(promises);

  await tempState.save();
};

const runProcessor = async () => {
  vftManagerDecoder = await Decoder.create('./assets/vft_manager.idl');
  hisotricalProxyDecoder = await Decoder.create('./assets/historical_proxy.idl');
  bridgingPaymentDecoder = await Decoder.create('./assets/bridging_payment.idl');

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
