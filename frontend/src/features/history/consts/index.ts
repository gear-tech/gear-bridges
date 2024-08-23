import EthSVG from '@/assets/eth.svg?react';
import VaraSVG from '@/assets/vara.svg?react';
import { NETWORK_NAME } from '@/consts';

import CheckSVG from '../assets/check.svg?react';
import ClockSVG from '../assets/clock.svg?react';
import { Direction, Status } from '../types';

import { TELEPORTS_QUERY } from './queries';

const INDEXER_ADDRESS = import.meta.env.VITE_INDEXER_ADDRESS as string;

const REFETCH_INTERVAL = 10000;
const LATEST_TRANSACTIONS_LIMIT = 5;
const TRANSACTIONS_LIMIT = 12;

const NETWORK_NAME_DIRECTION = {
  [NETWORK_NAME.VARA]: Direction.VaraToEth,
  [NETWORK_NAME.ETH]: Direction.EthToVara,
};

const DIRECTION_NETWORK_NAME = {
  [Direction.VaraToEth]: NETWORK_NAME.VARA,
  [Direction.EthToVara]: NETWORK_NAME.ETH,
} as const;

const DIRECTION_NETWORK_SVG = {
  [Direction.VaraToEth]: VaraSVG,
  [Direction.EthToVara]: EthSVG,
} as const;

const STATUS_SVG = {
  [Status.Completed]: CheckSVG,
  [Status.InProgress]: ClockSVG,
} as const;

export {
  INDEXER_ADDRESS,
  TELEPORTS_QUERY,
  REFETCH_INTERVAL,
  TRANSACTIONS_LIMIT,
  LATEST_TRANSACTIONS_LIMIT,
  NETWORK_NAME_DIRECTION,
  DIRECTION_NETWORK_NAME,
  DIRECTION_NETWORK_SVG,
  STATUS_SVG,
};
