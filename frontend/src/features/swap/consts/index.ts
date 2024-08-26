import { ABI, FUNGIBLE_TOKEN_ABI } from './abi';
import { FUNCTION_NAME, EVENT_NAME } from './eth';
import { FIELD_NAME, DEFAULT_VALUES, ADDRESS_SCHEMA, ERROR_MESSAGE } from './form';
import { BridgingPaymentProgram, VftGatewayProgram, VftProgram } from './sails';
import { TOKEN_TYPE, IDL_URL } from './spec';
import { STATE_FUNCTION } from './vara';

const BALANCE_REFETCH_INTERVAL = 10000;

export {
  ABI,
  FUNGIBLE_TOKEN_ABI,
  FIELD_NAME,
  DEFAULT_VALUES,
  ADDRESS_SCHEMA,
  FUNCTION_NAME,
  EVENT_NAME,
  ERROR_MESSAGE,
  BALANCE_REFETCH_INTERVAL,
  STATE_FUNCTION,
  TOKEN_TYPE,
  IDL_URL,
  BridgingPaymentProgram,
  VftGatewayProgram,
  VftProgram,
};
