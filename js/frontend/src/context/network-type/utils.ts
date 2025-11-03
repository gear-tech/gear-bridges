import * as ethNetworks from '@reown/appkit/networks';

import { NETWORK_SEARCH_PARAM, NETWORK_TYPE } from './consts';

const getEthNetwork = (id: number) => {
  const result = Object.values(ethNetworks)
    .filter((network) => 'id' in network)
    .find((network) => network.id === id);

  if (!result) throw new Error(`Chain with id ${id} not found`);

  return result;
};

function getNetworkEnv<T = string>(name: string): { MAINNET: T; TESTNET: T };
function getNetworkEnv<T>(name: string, format: (value: string) => T): { MAINNET: T; TESTNET: T };
function getNetworkEnv<T>(name: string, format?: (value: string) => T) {
  const value = import.meta.env[`VITE_${name}`] as string | undefined;

  if (!value) throw new Error(`Environment variable ${name} is not defined`);

  const [MAINNET, TESTNET] = value.split(',');

  if (!MAINNET) throw new Error(`Env ${name} is missing Mainnet value: ${value}`);
  if (!TESTNET) throw new Error(`Env ${name} is missing Testnet value: ${value}`);

  if (format) return { MAINNET: format(MAINNET), TESTNET: format(TESTNET) } as const;

  return { MAINNET, TESTNET } as const;
}

const getNetworkTypeFromUrl = (params = new URLSearchParams(window.location.search)) => {
  const network = params.get(NETWORK_SEARCH_PARAM);

  if (network !== NETWORK_TYPE.MAINNET && network !== NETWORK_TYPE.TESTNET) return;

  return network;
};

export { getEthNetwork, getNetworkEnv, getNetworkTypeFromUrl };
