import { BRIDGING_PAYMENT_ABI, ERC20_MANAGER_ABI, ERC20PERMIT_NONCES_ABI, ERC5267_ABI } from './abi';
import { CONTRACT_ADDRESS, ETH_BEACON_NODE_ADDRESS } from './env';
import { EVENT_NAME } from './eth';
import { FIELD_NAME, DEFAULT_VALUES, ADDRESS_SCHEMA, ERROR_MESSAGE, SUBMIT_STATUS } from './form';
import {
  BridgingPaymentProgram,
  VftManagerProgram,
  HistoricalProxyProgram,
  EthEventsProgram,
  CheckpointClientProgram,
} from './sails';
import { PRIORITY, CLAIM_TYPE } from './settings';
import { SERVICE_NAME, QUERY_NAME } from './vara';

const NETWORK = {
  VARA: 'vara',
  ETH: 'eth',
} as const;

export {
  CONTRACT_ADDRESS,
  BRIDGING_PAYMENT_ABI,
  ERC20_MANAGER_ABI,
  ERC20PERMIT_NONCES_ABI,
  ERC5267_ABI,
  FIELD_NAME,
  DEFAULT_VALUES,
  ADDRESS_SCHEMA,
  EVENT_NAME,
  ERROR_MESSAGE,
  NETWORK,
  SERVICE_NAME,
  QUERY_NAME,
  SUBMIT_STATUS,
  PRIORITY,
  CLAIM_TYPE,
  ETH_BEACON_NODE_ADDRESS,
  BridgingPaymentProgram,
  VftManagerProgram,
  HistoricalProxyProgram,
  EthEventsProgram,
  CheckpointClientProgram,
};
