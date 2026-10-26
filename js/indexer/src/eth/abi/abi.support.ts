import assert from 'assert';
import * as ethers from 'ethers';

export interface LogRecord {
  topics: string[];
  data: string;
  transactionHash?: string;
  blockHash?: string;
  logIndex?: string | number;
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
    try {
      const decoded = this.abi.decodeEventLog(this.fragment, rec.data, rec.topics);
      const canonical = this.abi.encodeEventLog(this.fragment, decoded);
      if (canonical.data.toLowerCase() !== rec.data.toLowerCase() ||
          canonical.topics.length !== rec.topics.length ||
          canonical.topics.some((topic, index) => topic.toLowerCase() !== rec.topics[index].toLowerCase())) {
        throw new Error('noncanonical or trailing event payload');
      }
      return decoded as unknown as Args;
    } catch (cause) {
      throw new Error(`HOLD: malformed ${this.fragment.name} at ${rec.blockHash ?? 'unknown block'}:${rec.transactionHash ?? 'unknown transaction'}:${rec.logIndex ?? 'unknown log'}`, { cause });
    }
  }
}
