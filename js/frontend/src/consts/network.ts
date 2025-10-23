const NETWORK_TYPE = {
  MAINNET: 'mainnet',
  TESTNET: 'testnet',
} as const;

const networkType = import.meta.env.VITE_NETWORK_TYPE as (typeof NETWORK_TYPE)[keyof typeof NETWORK_TYPE];

const NETWORK_TYPE_TO_NETWORK_NAME = {
  [NETWORK_TYPE.MAINNET]: {
    VARA: 'Vara Mainnet',
    ETH: 'Ethereum Mainnet',
  },

  [NETWORK_TYPE.TESTNET]: {
    VARA: 'Vara Testnet',
    ETH: 'Ethereum Hoodi',
  },
} as const;

const NETWORK_TYPE_TO_ETH_EXPLORER_URL = {
  [NETWORK_TYPE.MAINNET]: 'https://etherscan.io',
  [NETWORK_TYPE.TESTNET]: 'https://hoodi.etherscan.io',
} as const;

const NETWORK_NAME = NETWORK_TYPE_TO_NETWORK_NAME[networkType];
const ETH_EXPLORER_URL = NETWORK_TYPE_TO_ETH_EXPLORER_URL[networkType];

export { NETWORK_TYPE, networkType, NETWORK_NAME, ETH_EXPLORER_URL };
