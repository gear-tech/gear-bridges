import { useAlert, useApi } from '@gear-js/react-hooks';
import { useAppKitNetwork } from '@reown/appkit/react';
import { useEffect, useState } from 'react';
import { useSearchParams } from 'react-router-dom';

import { NETWORK_PRESET, NETWORK_TYPE } from '@/consts';
import { getNetwork } from '@/providers';
import { logger } from '@/utils';

import { Dropdown } from '../dropdown';

type NetworkType = (typeof NETWORK_TYPE)[keyof typeof NETWORK_TYPE];

const NETWORK_SEARCH_PARAM = 'network';
const NETWORK_LOCAL_STORAGE_KEY = 'networkType';

const getNetworkTypeFromUrl = (params = new URLSearchParams(window.location.search)) => {
  const network = params.get(NETWORK_SEARCH_PARAM);

  if (network !== NETWORK_TYPE.MAINNET && network !== NETWORK_TYPE.TESTNET) return;

  return network;
};

const DEFAULT_NETWORK_TYPE =
  getNetworkTypeFromUrl() || (localStorage[NETWORK_LOCAL_STORAGE_KEY] as string | null) || NETWORK_TYPE.MAINNET;

function NetworkSwitch() {
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

  const handleChange = (value: NetworkType) => {
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

  return <Dropdown value={networkType} onChange={handleChange} />;
}

export { NetworkSwitch };
