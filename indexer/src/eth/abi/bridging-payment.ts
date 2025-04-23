import ABI_JSON from '../../../assets/IBridgingPayment.json';
import { LogEvent } from './abi.support';
import * as ethers from 'ethers';

export const abi = new ethers.Interface(ABI_JSON.abi);

export const events = {
  FeePaid: new LogEvent<[]>(abi, abi.getEvent('FeePaid')!.topicHash),
};
