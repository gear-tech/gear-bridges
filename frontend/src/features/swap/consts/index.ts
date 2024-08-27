import { ERC20_TREASURY_ABI, FUNGIBLE_TOKEN_ABI } from './abi';
import { FUNCTION_NAME, EVENT_NAME } from './eth';
import { FIELD_NAME, DEFAULT_VALUES, ADDRESS_SCHEMA, ERROR_MESSAGE } from './form';
import { BridgingPaymentProgram, VftGatewayProgram, VftProgram } from './sails';

const NETWORK_INDEX = {
  VARA: 0,
  ETH: 1,
};

const BALANCE_REFETCH_INTERVAL = 10000;

export {
  ERC20_TREASURY_ABI,
  FUNGIBLE_TOKEN_ABI,
  FIELD_NAME,
  DEFAULT_VALUES,
  ADDRESS_SCHEMA,
  FUNCTION_NAME,
  EVENT_NAME,
  ERROR_MESSAGE,
  BALANCE_REFETCH_INTERVAL,
  NETWORK_INDEX,
  BridgingPaymentProgram,
  VftGatewayProgram,
  VftProgram,
};
