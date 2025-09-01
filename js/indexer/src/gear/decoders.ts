import { Decoder } from './codec.js';
import { config } from './config.js';
import { ProgramName } from './util.js';

let vftManagerDecoder: Decoder;
let historicalProxyDecoder: Decoder;
let bridgingPaymentDecoder: Decoder;
let vftDecoder: Decoder;
let checkpointClientDecoder: Decoder;

export async function initDecoders() {
  vftManagerDecoder = await Decoder.create(`${config.apiPath}/vft_manager.idl`);
  historicalProxyDecoder = await Decoder.create(`${config.apiPath}/historical_proxy.idl`);
  bridgingPaymentDecoder = await Decoder.create(`${config.apiPath}/bridging_payment.idl`);
  vftDecoder = await Decoder.create(`${config.apiPath}/vft.idl`);
  checkpointClientDecoder = await Decoder.create(`${config.apiPath}/checkpoint_light_client.idl`);
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
    case 'checkpoint_client':
      return checkpointClientDecoder;
    default:
      throw new Error(`Unknown program name: ${name}`);
  }
}
