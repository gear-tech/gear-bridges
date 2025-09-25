import { TypeormDatabase } from '@subsquid/typeorm-store';

import { isEthBridgeMessageQueued, isMessageQueued, isProgramChanged, isUserMessageSent, ProgramName } from './util.js';
import { ProcessorContext, getProcessor } from './processor.js';
import { getDecoder, initDecoders } from './decoders.js';
import { programs, setPrograms } from './programIds.js';
import { BatchState } from './batch-state.js';
import {
  handleVftManagerEvents,
  handleVftManagerInMsg,
  handleHistoricalProxyEvents,
  handleBridgingPaymentEvents,
  handleCheckpointClientEvents,
  handleEthBridgeMessage,
  handleProgramChangedEvent,
} from './handlers/index.js';

const state = new BatchState();

const handler = async (ctx: ProcessorContext) => {
  await state.new(ctx);

  for (const block of ctx.blocks) {
    for (const event of block.events) {
      if (isProgramChanged(event)) {
        handleProgramChangedEvent({
          state,
          blockHeader: block.header,
          event,
          log: ctx.log,
          rpc: ctx._chain.rpc,
        });
        continue;
      }

      if (isUserMessageSent(event)) {
        if (!programs.has(event.args.message.source)) continue;

        const name = programs.get(event.args.message.source);

        if (!name) {
          ctx.log.error(`Failed to get program name and decoder for ${event.args.message.source}`);
          continue;
        }

        const decoder = getDecoder(name);

        const eventCtx = {
          state,
          blockHeader: block.header,
          service: decoder.service(event.args.message.payload),
          method: decoder.method(event.args.message.payload),
          decoder,
          event,
          log: ctx.log,
        };

        if (name === ProgramName.VftManager) await handleVftManagerEvents(eventCtx);
        else if (name === ProgramName.HistoricalProxy) handleHistoricalProxyEvents(eventCtx);
        else if (name === ProgramName.BridgingPayment) handleBridgingPaymentEvents(eventCtx);
        else if (name === ProgramName.CheckpointClient) handleCheckpointClientEvents(eventCtx);
        else ctx.log.error(`Unknown program name ${name}`);

        continue;
      }

      if (isMessageQueued(event)) {
        if (!event.call) {
          ctx.log.error({ event }, 'Event call is undefined');
          continue;
        }

        if (event.call!.name !== `Gear.send_message`) continue;

        const name = programs.get(event.args.destination);

        if (name === ProgramName.VftManager) {
          const decoder = getDecoder(name);

          await handleVftManagerInMsg({
            state,
            event,
            decoder,
            blockHeader: block.header,
            log: ctx.log,
          });
        }
        continue;
      }

      if (isEthBridgeMessageQueued(event)) {
        handleEthBridgeMessage({
          state,
          event,
          blockHeader: block.header,
          log: ctx.log,
        });
        continue;
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
