import { useApi, useAlert } from '@gear-js/react-hooks';
import { useAppKitNetwork } from '@reown/appkit/react';
import { createContext, PropsWithChildren, useContext, useEffect, useState } from 'react';
import { useSearchParams } from 'react-router-dom';

import { NETWORK_PRESET, NETWORK_TYPE } from '@/consts';
import { getNetwork } from '@/providers';
import { logger } from '@/utils';

type Value = {
  networkType: NetworkType;
  switchNetworks: (value: NetworkType) => void;
};

const Context = createContext<Value | undefined>(undefined);
const { Provider } = Context;

const useNetworkType = () => {
  const context = useContext(Context);

  if (!context) throw new Error('useNetworkType must be used within a NetworkTypeProvider');

  return context;
};

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

function NetworkTypeProvider({ children }: PropsWithChildren) {
  const { switchNetwork } = useApi();
  const ethNetwork = useAppKitNetwork();
  const [searchParams, setSearchParams] = useSearchParams();
  const alert = useAlert();

  const [networkType, setNetworkType] = useState(DEFAULT_NETWORK_TYPE);

  useEffect(() => {
    if (getNetworkTypeFromUrl(searchParams)) return;

    searchParams.set(NETWORK_SEARCH_PARAM, networkType);
    setSearchParams(searchParams, { replace: true });
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [searchParams]);

  const switchNetworks = (value: NetworkType) => {
    const PRESET = NETWORK_PRESET[value.toUpperCase() as keyof typeof NETWORK_PRESET];

    setNetworkType(value);

    searchParams.set(NETWORK_SEARCH_PARAM, value);
    setSearchParams(searchParams);

    localStorage.setItem(NETWORK_LOCAL_STORAGE_KEY, value);

    Promise.all([
      switchNetwork({ endpoint: PRESET.NODE_ADDRESS }),
      ethNetwork.switchNetwork(getNetwork(PRESET.ETH_CHAIN_ID)),
    ]).catch((error: Error) => {
      alert.error(`Failed to switch network. ${error.message}`);
      logger.error('Network switch', error);
    });
  };

  return <Provider value={{ networkType, switchNetworks }}>{children}</Provider>;
}

// eslint-disable-next-line react-refresh/only-export-components
export { NetworkTypeProvider, useNetworkType };
