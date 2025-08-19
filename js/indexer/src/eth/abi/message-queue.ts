import { LogEvent } from './abi.support';
import * as ethers from 'ethers';
import * as fs from 'fs';
import { config } from '../config';

const ABI_JSON = JSON.parse(fs.readFileSync(`${config.apiPath}IMessageQueue.json`, 'utf-8'));

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
