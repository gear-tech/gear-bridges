import { TypeormDatabase } from '@subsquid/typeorm-store';
import { processor } from './processor.js';
import { handleBatch } from './handler.js';
import { config } from './config.js';

console.log(`Erc20Manager address: ${config.erc20Manager}`);
console.log(`BridgingPayment address: ${config.bridgingPayment}`);
console.log(`MessageQueue address: ${config.msgQ}`);

// Do not switch from the conservative cold-ingestion buffer to unfinalized hot heads.
processor.run(new TypeormDatabase({ supportHotBlocks: false, stateSchema: 'eth_processor' }), handleBatch);
