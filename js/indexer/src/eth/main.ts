import { TypeormDatabase } from '@subsquid/typeorm-store';
import { processor } from './processor.js';
import { handleBatch } from './handler.js';
import { config } from './config.js';

console.log(`Erc20Manager address: ${config.erc20Manager}`);
console.log(`BridgingPayment address: ${config.bridgingPayment}`);
console.log(`MessageQueue address: ${config.msgQ}`);

processor.run(new TypeormDatabase({ supportHotBlocks: true, stateSchema: 'eth_processor' }), handleBatch);
