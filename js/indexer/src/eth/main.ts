import { TypeormDatabase } from '@subsquid/typeorm-store';
import { randomUUID } from 'node:crypto';

import * as erc20TreasuryAbi from './abi/erc20-manager';
import * as messageQueueAbi from './abi/message-queue';
import * as bridgingPayment from './abi/bridging-payment';
import { Network, Status, Transfer } from '../model';
import { processor, Context } from './processor';
import { ethNonce, gearNonce } from '../common';
import { config } from './config';
import { BatchState } from './batch-state';

const state = new BatchState(Network.Ethereum, Network.Vara);

const ERC20_MANAGER = config.erc20Manager.toLowerCase();
const ERC20_MANAGER_BRIDGING_REQUESTED = erc20TreasuryAbi.events.BridgingRequested.topic;
const MSGQ = config.msgQ.toLowerCase();
const MSGQ_MESSAGE_PROCESSED = messageQueueAbi.events.MessageProcessed.topic;
const BRIDGING_PAYMENT = config.bridgingPayment.toLowerCase();
const BRIDGING_PAYMENT_FEE_PAID = bridgingPayment.events.FeePaid.topic;

console.log(`Erc20Manager address: ${ERC20_MANAGER}`);
console.log(`BridginPayment address: ${BRIDGING_PAYMENT}`);
console.log(`MessageQueue address: ${MSGQ}`);

const handler = async (ctx: Context) => {
  await state.new(ctx);

  for (const block of ctx.blocks) {
    const timestamp = new Date(block.header.timestamp);
    const blockNumber = BigInt(block.header.height);
    for (const log of block.logs) {
      const address = log.address.toLowerCase();
      const topic = log.topics[0].toLowerCase();
      const txHash = log.transactionHash.toLowerCase();
      switch (address) {
        case ERC20_MANAGER: {
          if (topic !== ERC20_MANAGER_BRIDGING_REQUESTED) continue;
          const [from, to, token, amount] = erc20TreasuryAbi.events.BridgingRequested.decode(log);

          const transfer = new Transfer({
            id: randomUUID(),
            txHash,
            blockNumber,
            timestamp,
            nonce: ethNonce(`${block.header.height}${log.transactionIndex}`),
            sourceNetwork: Network.Ethereum,
            source: token,
            destNetwork: Network.Vara,
            status: Status.AwaitingPayment,
            sender: from,
            receiver: to,
            amount,
          });
          await state.addTransfer(transfer);
          break;
        }
        case MSGQ: {
          if (topic !== MSGQ_MESSAGE_PROCESSED) continue;
          const [_, __, nonce, receiver] = messageQueueAbi.events.MessageProcessed.decode(log);
          if (receiver.toLowerCase() !== ERC20_MANAGER) continue;
          const _nonce = gearNonce(nonce, false);
          state.setCompletedTransfer(_nonce, timestamp, blockNumber, txHash);
          break;
        }
        case BRIDGING_PAYMENT: {
          if (topic !== BRIDGING_PAYMENT_FEE_PAID) continue;

          state.bridgingPaid(txHash);
        }
      }
    }
  }

  await state.save();
};

processor.run(new TypeormDatabase({ supportHotBlocks: true, stateSchema: 'eth_processor' }), handler);
