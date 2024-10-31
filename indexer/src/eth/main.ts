import { TypeormDatabase } from '@subsquid/typeorm-store';
import { randomUUID } from 'node:crypto';

import * as erc20TreasuryAbi from './abi/erc20-treasury';
import * as messageQueueAbi from './abi/message-queue';
import { Network, Status, Transfer } from '../model';
import { processor, Context } from './processor';
import { ethNonce, gearNonce, TempState } from '../common';
import { config } from './config';

const tempState = new TempState('eth');

const ERC20_TREASURY = config.erc20Treasury;
const ERC20_TREASURY_DEPOSIT = erc20TreasuryAbi.events.Deposit.topic;
const MSGQ = config.msgQ;
const MSGQ_MESSAGE_PROCESSED = messageQueueAbi.events.MessageProcessed.topic;

const handler = async (ctx: Context) => {
  await tempState.new(ctx);

  const promises = [];

  for (let block of ctx.blocks) {
    for (let log of block.logs) {
      const address = log.address.toLowerCase();
      const topic = log.topics[0].toLowerCase();
      if (address === ERC20_TREASURY && topic === ERC20_TREASURY_DEPOSIT) {
        const [from, to, token, amount] = erc20TreasuryAbi.events.Deposit.decode(log);

        tempState.transferRequested(
          new Transfer({
            id: randomUUID(),
            txHash: log.transactionHash,
            blockNumber: block.header.height.toString(),
            timestamp: new Date(block.header.timestamp),
            nonce: ethNonce(`${block.header.height}${log.transactionIndex}`),
            sourceNetwork: Network.Ethereum,
            source: token,
            destNetwork: Network.Gear,
            destination: tempState.getDestinationAddress(token),
            status: Status.Pending,
            sender: from,
            receiver: to,
            amount,
          }),
        );
      } else if (address === MSGQ && topic === MSGQ_MESSAGE_PROCESSED) {
        const [_, __, nonce] = messageQueueAbi.events.MessageProcessed.decode(log);
        promises.push(tempState.transferCompleted(gearNonce(nonce, false)));
      }
    }
  }

  await Promise.all(promises);

  await tempState.save();
};

processor.run(new TypeormDatabase({ supportHotBlocks: true, stateSchema: 'eth_processor' }), handler);
