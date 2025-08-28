import { LogEvent } from './abi.support';
import * as ethers from 'ethers';
import * as fs from 'fs';
import { config } from '../config';

const ABI_JSON = JSON.parse(fs.readFileSync(`${config.apiPath}IBridgingPayment.json`, 'utf-8'));

export const abi = new ethers.Interface(ABI_JSON.abi);

export const events = {
  FeePaid: new LogEvent<[]>(abi, abi.getEvent('FeePaid')!.topicHash),
};
