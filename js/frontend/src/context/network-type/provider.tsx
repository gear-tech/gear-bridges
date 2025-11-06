import { useApi, useAlert } from '@gear-js/react-hooks';
import { useAppKitNetwork } from '@reown/appkit/react';
import { PropsWithChildren, useState, useEffect, useMemo } from 'react';
import { useSearchParams } from 'react-router-dom';
import { useChainId, useConfig } from 'wagmi';

import { useEthAccount } from '@/hooks';
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
  const ethAccount = useEthAccount();

  useEffect(() => {
    logger.info('AppKit Network Chain ID: ', appKitNetwork.chainId);
    logger.info('Wagmi Chain ID: ', wagmiChainId);
    logger.info('Connector Chain ID: ', ethAccount.chainId);
  }, [appKitNetwork.chainId, wagmiChainId, ethAccount.chainId]);
}

function NetworkTypeProvider({ children }: PropsWithChildren) {
  const { isApiReady, switchNetwork } = useApi();
  const appKitNetwork = useAppKitNetwork();
  const config = useConfig();
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

  // useSwitchChain switches connector's network if wallet is connected,
  // so setting config directly to avoid it
  const switchWagmiNetwork = (chainId: number) => {
    config.setState((prevConfig) => ({ ...prevConfig, chainId }));
  };

  const switchNetworks = (value: NetworkType) => {
    const NEXT_PRESET = NETWORK_PRESET[value.toUpperCase() as keyof typeof NETWORK_PRESET];

    setNetworkType(value);
    switchWagmiNetwork(NEXT_PRESET.ETH_CHAIN_ID);

    searchParams.set(NETWORK_SEARCH_PARAM, value);
    setSearchParams(searchParams);

    localStorage.setItem(NETWORK_LOCAL_STORAGE_KEY, value);

    Promise.all([
      switchNetwork({ endpoint: NEXT_PRESET.NODE_ADDRESS }),
      appKitNetwork.switchNetwork(NEXT_PRESET.ETH_NETWORK),
    ]).catch((error: Error) => {
      alert.error(`Failed to switch network. ${error.message}`);
      logger.error('Network switch', error);
    });
  };

  const value = useMemo(
    () => ({ networkType, isMainnet, isTestnet, isLoading, NETWORK_PRESET: PRESET, switchNetworks }),
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [networkType, searchParams, config, isLoading],
  );

  return (
    <Provider value={value}>
      {children}
      <UnsupportedNetworkModal />
    </Provider>
  );
}

export { NetworkTypeProvider };
