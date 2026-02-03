import { ROUTE } from '@/consts';
import { NETWORK_TYPE } from '@/context/network-type';

const LINKS = {
  [ROUTE.HOME]: 'Bridge',
  [ROUTE.TRANSACTIONS]: 'Transactions',

  [ROUTE.TOKEN_TRACKER]: {
    [NETWORK_TYPE.MAINNET]: 'My Tokens',
    [NETWORK_TYPE.TESTNET]: 'My Tokens & Faucet',
  },

  [ROUTE.FAQ]: 'FAQ',
} as const;

export { LINKS };
