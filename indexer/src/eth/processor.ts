import {
  BlockHeader,
  DataHandlerContext,
  EvmBatchProcessor,
  EvmBatchProcessorFields,
  Log as _Log,
  Transaction as _Transaction,
} from '@subsquid/evm-processor';
import { Store } from '@subsquid/typeorm-store';

import * as erc20TreasuryAbi from './abi/erc20-treasury';
import * as messageQueueAbi from './abi/message-queue';
import { config } from './config';

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
    address: [config.erc20Treasury],
    topic0: [erc20TreasuryAbi.events.Deposit.topic],
  })
  .addLog({
    address: [config.msgQ],
    topic0: [messageQueueAbi.events.MessageProcessed.topic],
  })
  .setBlockRange({
    from: config.fromBlock,
    to: 2643979,
  });

export type Fields = EvmBatchProcessorFields<typeof processor>;
export type Context = DataHandlerContext<Store, Fields>;
export type Block = BlockHeader<Fields>;
export type Log = _Log<Fields>;
export type Transaction = _Transaction<Fields>;
