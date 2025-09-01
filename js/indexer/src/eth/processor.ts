import {
  BlockHeader,
  DataHandlerContext,
  EvmBatchProcessor,
  EvmBatchProcessorFields,
  Log as _Log,
  Transaction as _Transaction,
} from '@subsquid/evm-processor';
import { Store } from '@subsquid/typeorm-store';

import * as erc20TreasuryAbi from './abi/erc20-manager.js';
import * as messageQueueAbi from './abi/message-queue.js';
import { config } from './config.js';

export const processor = new EvmBatchProcessor()
  .setGateway(config.archiveUrl)
  .setRpcEndpoint({
    url: config.rpcUrl,
    rateLimit: 10,
  })
  .setFinalityConfirmation(75)
  .setFields({
    log: {
      transactionHash: true,
    },
  })
  .addLog({
    address: [config.erc20Manager],
    topic0: [erc20TreasuryAbi.events.BridgingRequested.topic],
  })
  .addLog({
    address: [config.msgQ],
    topic0: [messageQueueAbi.events.MessageProcessed.topic],
  })
  .setBlockRange({
    from: config.fromBlock,
  });

export type Fields = EvmBatchProcessorFields<typeof processor>;
export type Context = DataHandlerContext<Store, Fields>;
export type Block = BlockHeader<Fields>;
export type Log = _Log<Fields>;
export type Transaction = _Transaction<Fields>;
