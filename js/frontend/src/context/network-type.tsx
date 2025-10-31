import { HexString } from '@gear-js/api';
import { useApi, useAlert } from '@gear-js/react-hooks';
import { AppKitNetwork } from '@reown/appkit/networks';
import { useAppKitNetwork } from '@reown/appkit/react';
import { createContext, PropsWithChildren, useContext, useEffect, useState } from 'react';
import { useSearchParams } from 'react-router-dom';

import { getEthNetwork, logger } from '@/utils';

type Value = {
  networkType: NetworkType;
  NETWORK_PRESET: (typeof NETWORK_PRESET)[keyof typeof NETWORK_PRESET];
  isMainnet: boolean;
  isTestnet: boolean;
  switchNetworks: (value: NetworkType) => void;
};

const Context = createContext<Value | undefined>(undefined);
const { Provider } = Context;

const useNetworkType = () => {
  const context = useContext(Context);

  if (!context) throw new Error('useNetworkType must be used within a NetworkTypeProvider');

  return context;
};

const NETWORK_TYPE = {
  MAINNET: 'mainnet',
  TESTNET: 'testnet',
} as const;

type NetworkType = (typeof NETWORK_TYPE)[keyof typeof NETWORK_TYPE];

const NETWORK_SEARCH_PARAM = 'network';
const NETWORK_LOCAL_STORAGE_KEY = 'networkType';

const getNetworkTypeFromUrl = (params = new URLSearchParams(window.location.search)) => {
  const network = params.get(NETWORK_SEARCH_PARAM);

  if (network !== NETWORK_TYPE.MAINNET && network !== NETWORK_TYPE.TESTNET) return;

  return network;
};

const DEFAULT_NETWORK_TYPE =
  getNetworkTypeFromUrl() || (localStorage[NETWORK_LOCAL_STORAGE_KEY] as NetworkType | null) || NETWORK_TYPE.MAINNET;

type Preset = {
  NODE_ADDRESS: string;
  ARCHIVE_NODE_ADDRESS: string;

  ETH_NODE_ADDRESS: string;
  ETH_BEACON_NODE_ADDRESS: string;
  ETH_CHAIN_ID: number;

  INDEXER_ADDRESS: string;

  BRIDGING_PAYMENT_CONTRACT_ADDRESS: HexString;
  VFT_MANAGER_CONTRACT_ADDRESS: HexString;

  ETH_BRIDGING_PAYMENT_CONTRACT_ADDRESS: HexString;
  ERC20_MANAGER_CONTRACT_ADDRESS: HexString;

  ETH_MESSAGE_QUEUE_CONTRACT_ADDRESS: HexString;
  CHECKPOINT_CLIENT_CONTRACT_ADDRESS: HexString;
  HISTORICAL_PROXY_CONTRACT_ADDRESS: HexString;

  NETWORK_NAME: { VARA: string; ETH: string };
  EXPLORER_URL: { VARA: string | undefined; ETH: string };

  ETH_NETWORK: AppKitNetwork;
};

const NETWORK_PRESET = {
  MAINNET: MAINNET_PRESET,
  TESTNET: TESTNET_PRESET,
} as const;

function NetworkTypeProvider({ children }: PropsWithChildren) {
  const { switchNetwork } = useApi();
  const ethNetwork = useAppKitNetwork();
  const [searchParams, setSearchParams] = useSearchParams();
  const alert = useAlert();

  const [networkType, setNetworkType] = useState(DEFAULT_NETWORK_TYPE);
  const isMainnet = networkType === NETWORK_TYPE.MAINNET;
  const isTestnet = networkType === NETWORK_TYPE.TESTNET;
  const PRESET = NETWORK_PRESET[networkType.toUpperCase() as keyof typeof NETWORK_PRESET];

  useEffect(() => {
    if (getNetworkTypeFromUrl(searchParams)) return;

    searchParams.set(NETWORK_SEARCH_PARAM, networkType);
    setSearchParams(searchParams, { replace: true });
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [searchParams]);

  const switchNetworks = (value: NetworkType) => {
    const NEXT_PRESET = NETWORK_PRESET[value.toUpperCase() as keyof typeof NETWORK_PRESET];

    setNetworkType(value);

    searchParams.set(NETWORK_SEARCH_PARAM, value);
    setSearchParams(searchParams);

    localStorage.setItem(NETWORK_LOCAL_STORAGE_KEY, value);

    Promise.all([
      switchNetwork({ endpoint: NEXT_PRESET.NODE_ADDRESS }),
      ethNetwork.switchNetwork(NEXT_PRESET.ETH_NETWORK),
    ]).catch((error: Error) => {
      alert.error(`Failed to switch network. ${error.message}`);
      logger.error('Network switch', error);
    });
  };

  return (
    <Provider value={{ networkType, isMainnet, isTestnet, NETWORK_PRESET: PRESET, switchNetworks }}>
      {children}
    </Provider>
  );
}

// eslint-disable-next-line react-refresh/only-export-components
export { NetworkTypeProvider, useNetworkType, NETWORK_PRESET, DEFAULT_NETWORK_TYPE };
