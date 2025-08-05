import { LogEvent } from './abi.support';
import * as ethers from 'ethers';
import * as fs from 'fs';
import { config } from '../config';

const ABI_JSON = JSON.parse(fs.readFileSync(`${config.apiPath}IERC20Manager.json`, 'utf-8'));

export const abi = new ethers.Interface(ABI_JSON.abi);

export const events = {
  BridgingRequested: new LogEvent<
    [from: string, to: string, token: string, amount: bigint] & {
      from: string;
      to: string;
      token: string;
      amount: bigint;
    }
  >(abi, abi.getEvent('BridgingRequested')!.topicHash),
};
