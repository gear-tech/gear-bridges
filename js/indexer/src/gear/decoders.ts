import { Decoder } from './codec';
import { config } from './config';
import { ProgramName } from './util';

let vftManagerDecoder: Decoder;
let historicalProxyDecoder: Decoder;
let bridgingPaymentDecoder: Decoder;
let vftDecoder: Decoder;

export async function initDecoders() {
  vftManagerDecoder = await Decoder.create(`${config.apiPath}/vft_manager.idl`);
  historicalProxyDecoder = await Decoder.create(`${config.apiPath}/historical_proxy.idl`);
  bridgingPaymentDecoder = await Decoder.create(`${config.apiPath}/bridging_payment.idl`);
  vftDecoder = await Decoder.create(`${config.apiPath}/vft.idl`);
}

export function getDecoder(name: ProgramName | 'vft') {
  switch (name) {
    case 'vft_manager':
      return vftManagerDecoder;
    case 'historical_proxy':
      return historicalProxyDecoder;
    case 'bridging_payment':
      return bridgingPaymentDecoder;
    case 'vft':
      return vftDecoder;
    default:
      throw new Error(`Unknown program name: ${name}`);
  }
}
