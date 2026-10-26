import * as fs from 'fs';
import { getFnNamePrefix, getServiceNamePrefix, Sails } from 'sails-js';
import { SailsIdlParser } from 'sails-js-parser';

export class PayloadDecodeError extends Error {}

export class Decoder {
  constructor(private sails: Sails) {}

  static async create(idlPath: string) {
    const parser = new SailsIdlParser();
    await parser.init();
    const sails = new Sails(parser);
    sails.parseIdl(fs.readFileSync(idlPath, 'utf-8'));

    return new Decoder(sails);
  }
  private canonical(types: unknown[], data: `0x${string}`, service: string, method: string): void {
    try {
      if (!/^0x(?:[0-9a-fA-F]{2})*$/.test(data)) throw new Error('invalid hex');
      const tuple = this.sails.registry.createType(
        `(String, String, ${types.map((type) => typeof type === 'string' ? type : JSON.stringify(type)).join(', ')})`,
        data,
      );
      if (tuple.toHex().toLowerCase() !== data.toLowerCase() ||
          this.service(data) !== service || this.method(data) !== method) {
        throw new Error('noncanonical or trailing payload');
      }
    } catch (cause) {
      throw new PayloadDecodeError(`Invalid ${service}.${method} payload`, { cause });
    }
  }


  decodeInput<T>(service: string, fn: string, data: `0x${string}`): T {
    const method = this.sails.services[service].functions[fn];
    this.canonical(method.args.map((arg) => arg.type), data, service, fn);
    return method.decodePayload<T>(data);
  }

  decodeOutput<T>(service: string, fn: string, data: `0x${string}`): T {
    const method = this.sails.services[service].functions[fn];
    this.canonical([method.returnType], data, service, fn);
    return method.decodeResult<T>(data);
  }

  decodeQueryOutput<T>(service: string, fn: string, data: `0x${string}`): T {
    const method = this.sails.services[service].queries[fn];
    this.canonical([method.returnType], data, service, fn);
    return method.decodeResult<T>(data);
  }

  decodeEvent<T>(service: string, method: string, data: `0x${string}`): T {
    const event = this.sails.services[service].events[method];
    this.canonical([event.type], data, service, method);
    return event.decode(data);
  }

  private prefixes(data: `0x${string}`): [string, string] {
    try {
      if (!/^0x(?:[0-9a-fA-F]{2})*$/.test(data)) throw new Error('invalid hex');
      const service = getServiceNamePrefix(data, true);
      if (service.bytesLength > (data.length - 2) / 2) throw new Error('truncated service');
      const method = getFnNamePrefix(data, true);
      const length = service.bytesLength + method.bytesLength;
      if (length > (data.length - 2) / 2) throw new Error('truncated method');
      const encoded = this.sails.registry.createType('(String,String)', [service.service, method.fn]).toHex();
      if (encoded.toLowerCase() !== data.slice(0, 2 + length * 2).toLowerCase()) throw new Error('noncanonical prefix');
      return [service.service, method.fn];
    } catch (cause) {
      throw new PayloadDecodeError('Invalid Sails payload prefix', { cause });
    }
  }

  service(data: `0x${string}`): string {
    return this.prefixes(data)[0];
  }

  method(data: `0x${string}`): string {
    return this.prefixes(data)[1];
  }

  encodeQueryInput(service: string, fn: string, data: any[]): string {
    return this.sails.services[service].queries[fn].encodePayload(...data);
  }
}
