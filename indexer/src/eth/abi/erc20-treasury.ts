import ABI_JSON from '../../../assets/IERC20Manager.json';
import { LogEvent } from './abi.support';
import * as ethers from 'ethers';

export const abi = new ethers.Interface(ABI_JSON);

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
