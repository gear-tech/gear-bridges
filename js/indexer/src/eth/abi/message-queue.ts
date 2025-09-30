import * as ethers from 'ethers';
import * as fs from 'fs';

import { LogEvent } from './abi.support.js';
import { config } from '../config.js';

const ABI_JSON = JSON.parse(fs.readFileSync(`${config.apiPath}/IMessageQueue.json`, 'utf-8'));

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
  MerkleRoot: new LogEvent<
    [blockNumber: bigint, merkleRoot: string] & {
      blockNumber: bigint;
      merkleRoot: string;
    }
  >(abi, abi.getEvent('MerkleRoot')!.topicHash),
};
