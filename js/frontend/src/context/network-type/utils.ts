import { immutableOwnedFixtureNetwork, type OwnedFixtureNetwork } from '@gear-js/bridge';
import * as ethNetworks from '@reown/appkit/networks';

import { NETWORK_LOCAL_STORAGE_KEY, NETWORK_SEARCH_PARAM, NETWORK_TYPE } from './consts';

const getEthNetwork = (id: number, ownedFixture?: { descriptor: string | undefined; rpcUrl: string }) => {
  if (id === 31337) {
    if (!ownedFixture?.descriptor) throw new Error('HOLD: chain 31337 requires independent owned fixture approval');
    const descriptor = immutableOwnedFixtureNetwork(JSON.parse(ownedFixture.descriptor) as OwnedFixtureNetwork, [
      ownedFixture.rpcUrl,
    ]);
    const url = new URL(ownedFixture.rpcUrl);
    if (!['http:', 'https:'].includes(url.protocol))
      throw new Error('HOLD: owned wallet chain requires numeric loopback HTTP RPC');
    return ethNetworks.defineChain({
      id: descriptor.ethereumChainId,
      chainNamespace: 'eip155',
      caipNetworkId: 'eip155:31337',
      name: 'Owned Ethereum Fixture',
      nativeCurrency: { name: 'Ether', symbol: 'ETH', decimals: 18 },
      rpcUrls: { default: { http: [ownedFixture.rpcUrl] } },
      testnet: true,
    });
  }
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

  const [MAINNET, TESTNET] = value.split(',').map((v) => v.trim());

  if (!MAINNET) throw new Error(`Env VITE_${name} is missing Mainnet value: ${value}`);
  if (!TESTNET) throw new Error(`Env VITE_${name} is missing Testnet value: ${value}`);

  if (format) return { MAINNET: format(MAINNET), TESTNET: format(TESTNET) } as const;

  return { MAINNET, TESTNET } as const;
}

const getNetworkTypeFromStorage = () => {
  const value = localStorage.getItem(NETWORK_LOCAL_STORAGE_KEY);

  if (value === NETWORK_TYPE.MAINNET || value === NETWORK_TYPE.TESTNET) return value;
};

const getNetworkTypeFromUrl = (params = new URLSearchParams(window.location.search)) => {
  const network = params.get(NETWORK_SEARCH_PARAM);

  if (network !== NETWORK_TYPE.MAINNET && network !== NETWORK_TYPE.TESTNET) return;

  return network;
};

export { getEthNetwork, getNetworkEnv, getNetworkTypeFromStorage, getNetworkTypeFromUrl };
