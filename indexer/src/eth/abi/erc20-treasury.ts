import ABI_JSON from '../../../assets/IERC20Treasury.json';
import { LogEvent } from './abi.support';
import * as ethers from 'ethers';

export const abi = new ethers.Interface(ABI_JSON);

export const events = {
  Deposit: new LogEvent<
    [from: string, to: string, token: string, amount: bigint] & {
      from: string;
      to: string;
      token: string;
      amount: bigint;
    }
  >(abi, abi.getEvent('Deposit')!.topicHash),
};
