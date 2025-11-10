import { getNetworkTypeFromStorage, getNetworkTypeFromUrl } from '../utils';

import { NETWORK_PRESET } from './preset';

const NETWORK_SEARCH_PARAM = 'network';
const NETWORK_LOCAL_STORAGE_KEY = 'networkType';

const NETWORK_TYPE = {
  MAINNET: 'mainnet',
  TESTNET: 'testnet',
} as const;

const DEFAULT_NETWORK_TYPE = getNetworkTypeFromUrl() || getNetworkTypeFromStorage() || NETWORK_TYPE.MAINNET;

const DEFAULT_NETWORK_PRESET = NETWORK_PRESET[DEFAULT_NETWORK_TYPE.toUpperCase() as keyof typeof NETWORK_PRESET];

export {
  NETWORK_PRESET,
  NETWORK_TYPE,
  NETWORK_SEARCH_PARAM,
  NETWORK_LOCAL_STORAGE_KEY,
  DEFAULT_NETWORK_TYPE,
  DEFAULT_NETWORK_PRESET,
};
