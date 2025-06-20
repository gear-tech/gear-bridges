import { Network } from '../types';

import { INDEXER_ADDRESS } from './env';
import { FIELD_NAME, DEFAULT_VALUES, TIMESTAMP_OPTIONS, STATUS_OPTIONS } from './filters';
import { NETWORK_SVG } from './icons';
import { TRANSACTIONS_LIMIT, TRANSFERS_QUERY } from './queries';

const EXPLORER_URL = {
  [Network.Vara]: 'https://vara.subscan.io',
  [Network.Ethereum]: 'https://holesky.etherscan.io',
} as const;

export {
  INDEXER_ADDRESS,
  TRANSFERS_QUERY,
  TRANSACTIONS_LIMIT,
  FIELD_NAME,
  DEFAULT_VALUES,
  TIMESTAMP_OPTIONS,
  STATUS_OPTIONS,
  NETWORK_SVG,
  EXPLORER_URL,
};
