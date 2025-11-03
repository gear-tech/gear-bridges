import { NetworkType } from '../types';
import { getNetworkTypeFromUrl } from '../utils';

import { NETWORK_PRESET } from './preset';

const NETWORK_SEARCH_PARAM = 'network';
const NETWORK_LOCAL_STORAGE_KEY = 'networkType';

const NETWORK_TYPE = {
  MAINNET: 'mainnet',
  TESTNET: 'testnet',
} as const;

const DEFAULT_NETWORK_TYPE =
  getNetworkTypeFromUrl() || (localStorage[NETWORK_LOCAL_STORAGE_KEY] as NetworkType | null) || NETWORK_TYPE.MAINNET;

export { NETWORK_PRESET, NETWORK_TYPE, NETWORK_SEARCH_PARAM, NETWORK_LOCAL_STORAGE_KEY, DEFAULT_NETWORK_TYPE };
