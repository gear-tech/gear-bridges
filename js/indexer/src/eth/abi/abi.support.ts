import assert from 'assert';
import * as ethers from 'ethers';

export interface LogRecord {
  topics: string[];
  data: string;
}

export class LogEvent<Args> {
  private fragment: ethers.EventFragment;

  constructor(
    private abi: ethers.Interface,
    public readonly topic: string,
  ) {
    const fragment = abi.getEvent(topic);
    assert(fragment != null, 'Missing fragment');
    this.fragment = fragment;
  }

  decode(rec: LogRecord): Args {
    return this.abi.decodeEventLog(this.fragment, rec.data, rec.topics) as any as Args;
  }
}
