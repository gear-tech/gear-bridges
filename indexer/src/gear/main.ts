import { TypeormDatabase } from '@subsquid/typeorm-store';
import { randomUUID } from 'crypto';
import { BridgingRequested, Relayed, TokenMapping } from './types';
import { ethNonce, gearNonce, TempState } from '../common';
import { ProcessorContext, processor } from './processor';
import { Network, Status, Transfer } from '../model';
import { isUserMessageSent } from './util';
import { config } from './config';
import { Codec } from './codec';

const tempState = new TempState(Network.Gear);

let vftManagerDecoder: Codec;
let erc20RelayDecoder: Codec;
let historicalProxyDecoder: Codec;

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
                  status: Status.Pending,
                  sender,
                  receiver,
                  amount: BigInt(amount),
                });
                tempState.transferRequested(transfer);
                break;
              }
              case 'TokenMappingAdded': {
                const { vara_token_id, eth_token_id } = vftManagerDecoder.decodeEvent<TokenMapping>(
                  service,
                  method,
                  msg.payload,
                );
                tempState.addPair(vara_token_id, eth_token_id);
                break;
              }
              case 'TokenMappingRemoved': {
                const { vara_token_id, eth_token_id } = vftManagerDecoder.decodeEvent<TokenMapping>(
                  service,
                  method,
                  msg.payload,
                );
                tempState.removePair(vara_token_id, eth_token_id);
                break;
              }
              default: {
                continue;
              }
            }
          }
          case config.historicalProxy: {
            const service = historicalProxyDecoder.service(msg.payload);
            if (service !== 'HistoricalProxy') continue;
            const method = historicalProxyDecoder.method(msg.payload);
            if (method !== 'Redirect') continue;

            const { block_number, transaction_index } = historicalProxyDecoder.decodeEvent<Relayed>(
              service,
              method,
              msg.payload
            );

            const nonce = ethNonce(`${block_number}${transaction_index}`);
            promises.push(tempState.transferCompleted(nonce));
            break;
          }
          case config.erc20Relay: {
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
  vftManagerDecoder = await Codec.create('./assets/vft_manager.idl');
  erc20RelayDecoder = await Codec.create('./assets/erc20_relay.idl');
  historicalProxyDecoder = await Codec.create('./assets/historical_proxy.idl');

  processor.run(
    new TypeormDatabase({
      supportHotBlocks: true,
      stateSchema: 'gear_processor',
    }),
    handler,
  );
};

runProcessor();
