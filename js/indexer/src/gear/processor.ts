import { Store } from '@subsquid/typeorm-store';
import {
  BlockHeader,
  DataHandlerContext,
  SubstrateBatchProcessor,
  SubstrateBatchProcessorFields,
  Event as _Event,
  Call as _Call,
  Extrinsic as _Extrinsic,
} from '@subsquid/substrate-processor';
import { hostname } from 'node:os';

import { config } from './config.js';
import { Database } from '@subsquid/util-internal-processor-tools';

const processor = new SubstrateBatchProcessor()
  .setGateway(config.archiveUrl)
  .setRpcEndpoint({
    url: config.rpcUrl,
    rateLimit: config.rateLimit,
    headers: {
      'User-Agent': hostname(),
    },
  })
  .setFinalityConfirmation(10)
  .setFields({
    extrinsic: {
      hash: true,
    },
    event: {
      args: true,
    },
    block: {
      timestamp: true,
    },
  })
  .setBlockRange({
    from: config.fromBlock,
  })
  .addEvent({ name: ['Gear.ProgramChanged'] })
  .addEvent({
    name: ['Gear.MessageQueued', 'GearEthBridge.MessageQueued', 'Gear.UserMessageSent'],
    extrinsic: true,
    call: true,
  });

export type Fields = SubstrateBatchProcessorFields<typeof processor>;
export type Block = BlockHeader<Fields>;
export type Event = _Event<Fields>;
export type Call = _Call<Fields>;
export type Extrinsic = _Extrinsic<Fields>;
export type ProcessorContext = DataHandlerContext<Store, Fields>;

export function getProcessor(): SubstrateBatchProcessor {
  return processor;
  // .addGearUserMessageSent({ programId: programIds, extrinsic: true, call: true });
}
