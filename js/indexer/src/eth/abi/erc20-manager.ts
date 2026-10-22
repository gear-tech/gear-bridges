import * as ethers from 'ethers';
import * as fs from 'fs';

import { LogEvent } from './abi.support.js';
import { getEnv } from '../../common/env.js';

const ABI_JSON = JSON.parse(fs.readFileSync(`${getEnv('ETH_API_PATH', './api/ethereum')}/IERC20Manager.json`, 'utf-8'));

export const abi = new ethers.Interface(ABI_JSON.abi);

export const events = {
  Bridged: new LogEvent<
    [from: string, to: string, token: string, amount: bigint]
  >(abi, abi.getEvent('Bridged')!.topicHash),
  BridgingRequested: new LogEvent<
    [from: string, to: string, token: string, amount: bigint] & {
      from: string;
      to: string;
      token: string;
      amount: bigint;
    }
  >(abi, abi.getEvent('BridgingRequested')!.topicHash),
};
