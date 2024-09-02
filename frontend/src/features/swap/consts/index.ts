import { HexString } from '@gear-js/api';

import { ERC20_TREASURY_ABI, FUNGIBLE_TOKEN_ABI } from './abi';
import { EVENT_NAME } from './eth';
import { FIELD_NAME, DEFAULT_VALUES, ADDRESS_SCHEMA, ERROR_MESSAGE } from './form';
import { BridgingPaymentProgram, VftGatewayProgram, VftProgram } from './sails';
import { SERVICE_NAME, QUERY_NAME } from './vara';

const BRIDGING_PAYMENT_CONTRACT_ADDRESS = import.meta.env.VITE_BRIDGING_PAYMENT_CONTRACT_ADDRESS as HexString;
const ERC20_TREASURY_CONTRACT_ADDRESS = import.meta.env.VITE_ERC20_TREASURY_CONTRACT_ADDRESS as HexString;

const NETWORK_INDEX = {
  VARA: 0,
  ETH: 1,
};

const BALANCE_REFETCH_INTERVAL = 10000;

export {
  BRIDGING_PAYMENT_CONTRACT_ADDRESS,
  ERC20_TREASURY_CONTRACT_ADDRESS,
  ERC20_TREASURY_ABI,
  FUNGIBLE_TOKEN_ABI,
  FIELD_NAME,
  DEFAULT_VALUES,
  ADDRESS_SCHEMA,
  EVENT_NAME,
  ERROR_MESSAGE,
  BALANCE_REFETCH_INTERVAL,
  NETWORK_INDEX,
  SERVICE_NAME,
  QUERY_NAME,
  BridgingPaymentProgram,
  VftGatewayProgram,
  VftProgram,
};
