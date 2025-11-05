import { useApi, useAlert } from '@gear-js/react-hooks';
import { useAppKitNetwork } from '@reown/appkit/react';
import { PropsWithChildren, useState, useEffect, useMemo } from 'react';
import { useSearchParams } from 'react-router-dom';
import { useChainId } from 'wagmi';

import { logger } from '@/utils';

import {
  DEFAULT_NETWORK_TYPE,
  NETWORK_PRESET,
  NETWORK_TYPE,
  NETWORK_SEARCH_PARAM,
  NETWORK_LOCAL_STORAGE_KEY,
} from './consts';
import { Provider } from './context';
import { NetworkType } from './types';
import { UnsupportedNetworkModal } from './unsupported-network-modal';
import { getNetworkTypeFromUrl } from './utils';

function useChainIdLogs() {
  const appKitNetwork = useAppKitNetwork();
  const wagmiChainId = useChainId();

  useEffect(() => {
    logger.info('AppKit Network Chain ID: ', appKitNetwork.chainId);
    logger.info('Wagmi Chain ID: ', wagmiChainId);
  }, [appKitNetwork.chainId, wagmiChainId]);
}

function NetworkTypeProvider({ children }: PropsWithChildren) {
  const { isApiReady, switchNetwork } = useApi();
  const ethNetwork = useAppKitNetwork();
  const isLoading = !isApiReady;

  useChainIdLogs();

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

  const value = useMemo(
    () => ({ networkType, isMainnet, isTestnet, isLoading, NETWORK_PRESET: PRESET, switchNetworks }),
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [networkType, searchParams, isLoading],
  );

  return (
    <Provider value={value}>
      {children}
      <UnsupportedNetworkModal />
    </Provider>
  );
}

export { NetworkTypeProvider };
