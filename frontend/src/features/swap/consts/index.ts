import { BRIDGING_PAYMENT_ABI } from './abi';
import { BRIDGING_PAYMENT_CONTRACT_ADDRESS, ETH_BRIDGING_PAYMENT_CONTRACT_ADDRESS } from './env';
import { EVENT_NAME } from './eth';
import { FIELD_NAME, DEFAULT_VALUES, ADDRESS_SCHEMA, ERROR_MESSAGE } from './form';
import { BridgingPaymentProgram, WrappedVaraProgram } from './sails';
import { SERVICE_NAME, QUERY_NAME } from './vara';

const NETWORK_INDEX = {
  VARA: 0,
  ETH: 1,
};

export {
  BRIDGING_PAYMENT_CONTRACT_ADDRESS,
  ETH_BRIDGING_PAYMENT_CONTRACT_ADDRESS,
  BRIDGING_PAYMENT_ABI,
  FIELD_NAME,
  DEFAULT_VALUES,
  ADDRESS_SCHEMA,
  EVENT_NAME,
  ERROR_MESSAGE,
  NETWORK_INDEX,
  SERVICE_NAME,
  QUERY_NAME,
  BridgingPaymentProgram,
  WrappedVaraProgram,
};
