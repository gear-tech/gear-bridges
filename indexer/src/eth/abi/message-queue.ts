import ABI_JSON from '../../../assets/IMessageQueue.json';
import { LogEvent } from './abi.support';
import * as ethers from 'ethers';

export const abi = new ethers.Interface(ABI_JSON.abi);

export const events = {
  MessageProcessed: new LogEvent<
    [blockNumber: bigint, messageHash: string, messageNonce: bigint, messageReceiver: string] & {
      blockNumber: bigint;
      messageHash: string;
      nonce: bigint;
      messageReceiver: string;
    }
  >(abi, abi.getEvent('MessageProcessed')!.topicHash),
};
