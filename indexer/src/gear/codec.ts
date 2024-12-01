import * as fs from 'fs';
import { getFnNamePrefix, getServiceNamePrefix, Sails } from 'sails-js';
import { SailsIdlParser } from 'sails-js-parser';

export class Codec {
  constructor(private sails: Sails) {}

  static async create(idlPath: string) {
    const parser = new SailsIdlParser();
    await parser.init();
    const vft = new Sails(parser);
    vft.parseIdl(fs.readFileSync(idlPath, 'utf-8'));

    return new Codec(vft);
  }

  decodeInput<T>(data: `0x${string}`): T {
    const service = getServiceNamePrefix(data);
    const fn = getFnNamePrefix(data);

    return this.sails.services[service].functions[fn].decodePayload<T>(data);
  }

  decodeOutput<T>(service: string, fn: string, data: `0x${string}`): T {
    return this.sails.services[service].functions[fn].decodeResult<T>(data);
  }

  decodeEvent<T>(service: string, method: string, data: `0x${string}`): T {
    return this.sails.services[service].events[method].decode(data);
  }

  service(data: `0x${string}`): string {
    return getServiceNamePrefix(data);
  }

  method(data: `0x${string}`): string {
    return getFnNamePrefix(data);
  }
}
