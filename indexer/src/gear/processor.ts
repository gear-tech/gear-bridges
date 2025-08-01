import {
  BlockHeader,
  DataHandlerContext,
  SubstrateBatchProcessor,
  SubstrateBatchProcessorFields,
  Event as _Event,
  Call as _Call,
  Extrinsic as _Extrinsic,
} from '@subsquid/substrate-processor';
import { Store } from '@subsquid/typeorm-store';
import { config } from './config';
import { hostname } from 'node:os';

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
  .addEvent({ name: ['Gear.MessageQueued', 'GearEthBridge.MessageQueued'], extrinsic: true, call: true });

export type Fields = SubstrateBatchProcessorFields<typeof processor>;
export type Block = BlockHeader<Fields>;
export type Event = _Event<Fields>;
export type Call = _Call<Fields>;
export type Extrinsic = _Extrinsic<Fields>;
export type ProcessorContext = DataHandlerContext<Store, Fields>;

export function getProcessor(programIds: string[]): SubstrateBatchProcessor {
  return processor.addGearUserMessageSent({ programId: programIds, extrinsic: true, call: true });
}
