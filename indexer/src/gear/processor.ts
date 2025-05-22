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

const processor = new SubstrateBatchProcessor()
  .setGateway(config.archiveUrl)
  .setRpcEndpoint({
    url: config.rpcUrl,
    rateLimit: 10,
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
  .addEvent({ name: ['Gear.ProgramChanged'] });

export type Fields = SubstrateBatchProcessorFields<typeof processor>;
export type Block = BlockHeader<Fields>;
export type Event = _Event<Fields>;
export type Call = _Call<Fields>;
export type Extrinsic = _Extrinsic<Fields>;
export type ProcessorContext = DataHandlerContext<Store, Fields>;

export function getProcessor(programIds: string[]): SubstrateBatchProcessor {
  return processor.addGearUserMessageSent({ programId: programIds, extrinsic: true, call: true });
}
