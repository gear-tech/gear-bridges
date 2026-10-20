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
  handleEthBridgeMessage,
  handleProgramChangedEvent,
} from './handlers/index.js';
import { config } from './config.js';
import { authenticateManagerRouting } from './manager-routing.js';
import { queryVftManagerPairs } from './rpc-queries.js';
import { GearManagerHandoff, Network, Pair } from '../model/index.js';
import { createPairHash } from 'gear-bridge-common';
import { requireFinalizedBatch } from './finality.js';
const state = new BatchState();

let isFirstRun = true;

const handler = async (ctx: ProcessorContext) => {
  await requireFinalizedBatch(ctx);
  await state.new(ctx);
  if (!ctx.blocks.length) return;
  const history = await ctx.store.find(GearManagerHandoff, {});
  const managerRoot = () => {
    const id = [...programs].find(([, name]) => name === ProgramName.VftManager)?.[0];
    if (!id) throw new Error('HOLD: authenticated manager root missing');
    return id;
  };
  let routing = await authenticateManagerRouting(ctx, ctx.blocks[0].header, managerRoot(), history);

  if (ctx.isHead && isFirstRun) {
    ctx.log.info('First run of the processor, syncing active pairs from VFT Manager');
    const activePairs = await queryVftManagerPairs(ctx._chain.rpc, routing.current, ctx.blocks[0].header.hash);

    const pairs = await ctx.store.find(Pair);

    const currentActivePairs = pairs.filter((p) => p.isActive).map((p) => p.id);
    ctx.log.info(
      { activePairsCount: activePairs.length, currentActivePairsCount: currentActivePairs.length },
      'Active pairs count',
    );
    const actualActivePairs: Set<string> = new Set();
    const currentInactivePairs = pairs.filter((p) => !p.isActive).map((p) => p.id);

    for (const pair of activePairs) {
      const id = createPairHash(pair[0].toLowerCase(), pair[1].toLowerCase());
      if (currentActivePairs.includes(id)) {
        ctx.log.info({ id }, 'Pair is already in the database as active');
        actualActivePairs.add(id);
        continue;
      }
      if (currentInactivePairs.includes(id)) {
        ctx.log.error({ id }, 'Pair is in the database as inactive, but active in VFT Manager');
        throw new Error(`Pair ${id} is already in the database as inactive`);
      }

      if (pair[2] !== 'Ethereum' && pair[2] !== 'Gear') {
        ctx.log.error({ id, supplyType: pair[2] }, 'Unknown supply type for pair');
        throw new Error(`Unknown supply type ${pair[2]} for pair ${id}`);
      }

      ctx.log.info({ id, vara: pair[0], eth: pair[1] }, 'Adding pair to the database as active');
      await state.addPair(
        pair[0],
        pair[1],
        pair[2] === 'Ethereum' ? Network.Ethereum : Network.Vara,
        ctx.blocks[0].header,
      );
    }

    if (actualActivePairs.size !== currentActivePairs.length) {
      ctx.log.info(
        { actualActivePairsCount: actualActivePairs.size, currentActivePairsCount: currentActivePairs.length },
        'Some pairs are no longer active according to VFT Manager, marking them as inactive',
      );

      const actualInactivePairs = currentActivePairs.filter((id) => !actualActivePairs.has(id));

      actualInactivePairs.forEach((id) => state.removePairById(id, BigInt(ctx.blocks[0].header.height)));
    }

    isFirstRun = false;
  }

  for (const block of ctx.blocks) {
    if (block !== ctx.blocks[0]) {
      routing = await authenticateManagerRouting(ctx, block.header, managerRoot(), history);
    }
    const retained = config.retainedCutoverFromBlock !== undefined && block.header.height >= config.retainedCutoverFromBlock;
    for (const event of block.events) {
      if (isProgramChanged(event)) {
        if (retained && routing.domains.has(event.args.id) && event.args.change.__kind === 'Inactive') {
          throw new Error('HOLD: retained manager unexpectedly exited');
        }
        await handleProgramChangedEvent({
          state,
          blockHeader: block.header,
          event,
          log: ctx.log,
          rpc: ctx._chain.rpc,
        });
        continue;
      }

      if (isUserMessageSent(event)) {
        const source = event.args.message.source;
        const name = retained && routing.domains.has(source) ? ProgramName.VftManager : programs.get(source);
        if (!name) continue;


        const decoder = getDecoder(name);

        const eventCtx = {
          state,
          blockHeader: block.header,
          service: decoder.service(event.args.message.payload),
          method: decoder.method(event.args.message.payload),
          decoder,
          event,
          log: ctx.log,
          receiptDomain: retained ? routing.domains.get(source) : undefined,
        };

        if (name === ProgramName.VftManager) await handleVftManagerEvents(eventCtx);
        else if (name === ProgramName.HistoricalProxy) handleHistoricalProxyEvents(eventCtx);
        else if (name === ProgramName.BridgingPayment) handleBridgingPaymentEvents(eventCtx);
        else ctx.log.error({ programName: name }, 'Unknown program name');

        continue;
      }

      if (isMessageQueued(event)) {
        if (!event.call) {
          ctx.log.error({ event }, 'Event call is undefined');
          continue;
        }

        if (event.call!.name !== `Gear.send_message`) continue;

        const name = retained && routing.domains.has(event.args.destination)
          ? ProgramName.VftManager : programs.get(event.args.destination);

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
    supportHotBlocks: false,
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
