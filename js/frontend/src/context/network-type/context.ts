import { createContext, useContext } from 'react';

import { NetworkPreset, NetworkType } from './types';

type Value = {
  NETWORK_PRESET: NetworkPreset;
  networkType: NetworkType;
  isMainnet: boolean;
  isTestnet: boolean;
  isLoading: boolean;
  switchNetworks: (value: NetworkType) => void;
};

const Context = createContext<Value | undefined>(undefined);

const { Provider } = Context;

const useNetworkType = () => {
  const context = useContext(Context);

  if (!context) throw new Error('useNetworkType must be used within a NetworkTypeProvider');

  return context;
};

export { Provider, useNetworkType };
