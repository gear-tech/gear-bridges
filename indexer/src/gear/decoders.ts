import { Decoder } from './codec';
import { ProgramName } from './util';

let vftManagerDecoder: Decoder;
let hisotricalProxyDecoder: Decoder;
let bridgingPaymentDecoder: Decoder;
let vftDecoder: Decoder;

export async function initDecoders() {
  vftManagerDecoder = await Decoder.create('./assets/vft_manager.idl');
  hisotricalProxyDecoder = await Decoder.create('./assets/historical_proxy.idl');
  bridgingPaymentDecoder = await Decoder.create('./assets/bridging_payment.idl');
  vftDecoder = await Decoder.create('./assets/vft.idl');
}

export function getDecoder(name: ProgramName | 'vft') {
  switch (name) {
    case 'vft_manager':
      return vftManagerDecoder;
    case 'historical_proxy':
      return hisotricalProxyDecoder;
    case 'bridging_payment':
      return bridgingPaymentDecoder;
    case 'vft':
      return vftDecoder;
    default:
      throw new Error(`Unknown program name: ${name}`);
  }
}
