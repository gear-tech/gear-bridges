import ABI_JSON from '../../../assets/IMessageQueue.json';
import { LogEvent } from './abi.support';
import * as ethers from 'ethers';

export const abi = new ethers.Interface(ABI_JSON.abi);

export const events = {
  MessageProcessed: new LogEvent<
    [blockNumber: bigint, messageHash: string, messageNonce: string, messageReceiver: string] & {
      blockNumber: bigint;
      messageHash: string;
      nonce: string;
      messageReceiver: string;
    }
  >(abi, abi.getEvent('MessageProcessed')!.topicHash),
};
