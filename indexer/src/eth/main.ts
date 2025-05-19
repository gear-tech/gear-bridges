import { TypeormDatabase } from '@subsquid/typeorm-store';
import { randomUUID } from 'node:crypto';

import * as erc20TreasuryAbi from './abi/erc20-manager';
import * as messageQueueAbi from './abi/message-queue';
import { Network, Status, Transfer } from '../model';
import { processor, Context } from './processor';
import { ethNonce, gearNonce, TempState } from '../common';
import { config } from './config';

const tempState = new TempState(Network.Ethereum);

const ERC20_MANAGER = config.erc20Manager;
const ERC20_MANAGER_BRIDGING_REQUESTED = erc20TreasuryAbi.events.BridgingRequested.topic;
const MSGQ = config.msgQ;
const MSGQ_MESSAGE_PROCESSED = messageQueueAbi.events.MessageProcessed.topic;

const handler = async (ctx: Context) => {
  await tempState.new(ctx);

  for (let block of ctx.blocks) {
    const timestamp = new Date(block.header.timestamp);
    for (let log of block.logs) {
      const address = log.address.toLowerCase();
      const topic = log.topics[0].toLowerCase();
      switch (address) {
        case ERC20_MANAGER: {
          if (topic !== ERC20_MANAGER_BRIDGING_REQUESTED) continue;
          const [from, to, token, amount] = erc20TreasuryAbi.events.BridgingRequested.decode(log);
          tempState.transferRequested(
            new Transfer({
              id: randomUUID(),
              txHash: log.transactionHash,
              blockNumber: block.header.height.toString(),
              timestamp,
              nonce: ethNonce(`${block.header.height}${log.transactionIndex}`),
              sourceNetwork: Network.Ethereum,
              source: token,
              destNetwork: Network.Gear,
              destination: tempState.getDestinationAddress(token),
              status: Status.Bridging,
              sender: from,
              receiver: to,
              amount,
            }),
          );
          break;
        }
        case MSGQ: {
          if (topic !== MSGQ_MESSAGE_PROCESSED) continue;
          const [_, __, nonce] = messageQueueAbi.events.MessageProcessed.decode(log);
          tempState.transferCompleted(gearNonce(nonce, false), timestamp);
          break;
        }
      }
    }
  }

  await tempState.save();
};

processor.run(new TypeormDatabase({ supportHotBlocks: true, stateSchema: 'eth_processor' }), handler);
