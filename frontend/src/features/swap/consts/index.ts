import { BRIDGING_PAYMENT_ABI, ERC20_MANAGER_ABI, USDC_ABI } from './abi';
import { CONTRACT_ADDRESS } from './env';
import { EVENT_NAME } from './eth';
import { FIELD_NAME, DEFAULT_VALUES, ADDRESS_SCHEMA, ERROR_MESSAGE, SUBMIT_STATUS } from './form';
import { BridgingPaymentProgram, VftManagerProgram } from './sails';
import { SERVICE_NAME, QUERY_NAME } from './vara';

const NETWORK = {
  VARA: 'vara',
  ETH: 'eth',
} as const;

export {
  CONTRACT_ADDRESS,
  BRIDGING_PAYMENT_ABI,
  ERC20_MANAGER_ABI,
  USDC_ABI,
  FIELD_NAME,
  DEFAULT_VALUES,
  ADDRESS_SCHEMA,
  EVENT_NAME,
  ERROR_MESSAGE,
  NETWORK,
  SERVICE_NAME,
  QUERY_NAME,
  SUBMIT_STATUS,
  BridgingPaymentProgram,
  VftManagerProgram,
};
