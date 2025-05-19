import { TypeormDatabase } from '@subsquid/typeorm-store';
import { randomUUID } from 'crypto';
import { BridgingPaidEvent, BridgingRequested, Relayed, TokenMappingAdded, TokenMappingRemoved } from './types';
import { ethNonce, gearNonce, TempState } from '../common';
import { ProcessorContext, processor } from './processor';
import { Network, Status, Transfer } from '../model';
import { isUserMessageSent } from './util';
import { config } from './config';
import { Decoder } from './codec';

const tempState = new TempState(Network.Gear);

let vftManagerDecoder: Decoder;
let hisotricalProxyDecoder: Decoder;
let bridgingPaymentDecoder: Decoder;

const handler = async (ctx: ProcessorContext) => {
  await tempState.new(ctx);

  const promises = [];

  for (const block of ctx.blocks) {
    const timestamp = new Date(block.header.timestamp!);
    const blockNumber = block.header.height.toString();

    for (const event of block.events) {
      if (isUserMessageSent(event)) {
        const msg = event.args.message;
        switch (msg.source) {
          case config.vftManager: {
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
                  destination: tempState.getDestinationAddress(vara_token_id),
                  status: Status.AwaitingPayment,
                  sender,
                  receiver,
                  amount: BigInt(amount),
                });
                tempState.transferRequested(transfer);
                break;
              }
              case 'TokenMappingAdded': {
                const { vara_token_id, eth_token_id, supply_type } = vftManagerDecoder.decodeEvent<TokenMappingAdded>(
                  service,
                  method,
                  msg.payload,
                );

                tempState.addPair(
                  vara_token_id.toLowerCase(),
                  eth_token_id.toLowerCase(),
                  supply_type === 'Ethereum' ? Network.Ethereum : Network.Gear,
                );
                break;
              }
              case 'TokenMappingRemoved': {
                const { vara_token_id, eth_token_id } = vftManagerDecoder.decodeEvent<TokenMappingRemoved>(
                  service,
                  method,
                  msg.payload,
                );
                promises.push(tempState.removePair(vara_token_id.toLowerCase(), eth_token_id.toLowerCase()));
                break;
              }
              default: {
                continue;
              }
            }
          }
          case config.hisotricalProxy: {
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
          case config.bridgingPayment: {
            const service = bridgingPaymentDecoder.service(msg.payload);
            if (service !== 'BridgingPayment') continue;
            const method = bridgingPaymentDecoder.method(msg.payload);
            if (method !== 'BridgingPaid') continue;

            const { nonce } = bridgingPaymentDecoder.decodeEvent<BridgingPaidEvent>(service, method, msg.payload);

            promises.push(tempState.transferStatus(gearNonce(nonce), Status.Bridging));
            break;
          }
        }
      }
    }
  }

  await Promise.all(promises);

  await tempState.save();
};

export const runProcessor = async () => {
  vftManagerDecoder = await Decoder.create('./assets/vft_manager.idl');
  hisotricalProxyDecoder = await Decoder.create('./assets/historical_proxy.idl');
  bridgingPaymentDecoder = await Decoder.create('./assets/bridging_payment.idl');

  processor.run(
    new TypeormDatabase({
      supportHotBlocks: true,
      stateSchema: 'gear_processor',
    }),
    handler,
  );
};

runProcessor();
