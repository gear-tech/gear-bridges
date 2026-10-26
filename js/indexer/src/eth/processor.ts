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
import * as bridgingPaymentAbi from './abi/bridging-payment.js';
import { config } from './config.js';

export const processor = new EvmBatchProcessor()
  .setRpcEndpoint({
    url: config.rpcUrl,
    rateLimit: config.rateLimit,
    requestTimeout: 15_000,
    retryAttempts: 2,
  })
  // Only an ingestion/scheduling buffer; the handler authenticates finalized hashes.
  .setFinalityConfirmation(75)
  .setFields({
    // Select Subsquid's receipt-based RPC ingestion: public providers reject its
    // unfiltered eth_getLogs request. Receipt/block consistency checks stay enabled.
    transaction: { status: true },
    log: {
      transactionHash: true,
    },
  })
  .addLog({
    address: [config.erc20Manager],
    transaction: true,
    topic0: [erc20TreasuryAbi.events.BridgingRequested.topic],
  })
  .addLog({
    address: [config.msgQ],
    transaction: true,
    topic0: [messageQueueAbi.events.MessageProcessed.topic, messageQueueAbi.events.MerkleRoot.topic],
  })
  .addLog({
    address: [config.bridgingPayment],
    transaction: true,
    topic0: [bridgingPaymentAbi.events.FeePaid.topic],
  })
  .setBlockRange({
    from: config.fromBlock,
  });

if (!config.rpcOnly) processor.setGateway({ url: config.archiveUrl, apiKey: config.apiKey });

export type Fields = EvmBatchProcessorFields<typeof processor>;
export type Context = DataHandlerContext<Store, Fields>;
export type Block = BlockHeader<Fields>;
export type Log = _Log<Fields>;
export type Transaction = _Transaction<Fields>;
