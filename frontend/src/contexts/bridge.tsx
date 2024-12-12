import { HexString } from '@gear-js/api';
import { createContext, PropsWithChildren, useContext, useEffect, useMemo, useState } from 'react';

import { useEthAccount, useTokens } from '@/hooks';

const NETWORK_INDEX = {
  VARA: 0,
  ETH: 1,
} as const;

type BridgeContextType = {
  tokenAddress: HexString | undefined;
  setTokenAddress: (address: HexString) => void;
  networkIndex: (typeof NETWORK_INDEX)[keyof typeof NETWORK_INDEX];
};

const DEFAULT_BRIDGE_CONTEXT: BridgeContextType = {
  tokenAddress: undefined,
  setTokenAddress: () => {},
  networkIndex: NETWORK_INDEX.VARA,
} as const;

const BridgeContext = createContext(DEFAULT_BRIDGE_CONTEXT);
const { Provider } = BridgeContext;
const useBridge = () => useContext(BridgeContext);

function BridgeProvider({ children }: PropsWithChildren) {
  const ethAccount = useEthAccount();

  // since eth account is reconnecting immediately without any visible loading state,
  // and in swap form vara is the first network by default,
  // check for loading status (isAccountReady || ethAccount.isReconnecting) is minor and can be neglected
  const networkIndex = ethAccount.isConnected ? NETWORK_INDEX.ETH : NETWORK_INDEX.VARA;

  const { addresses } = useTokens();
  const [tokenAddress, setTokenAddress] = useState<HexString>();

  useEffect(() => {
    if (!addresses) return;

    setTokenAddress(addresses[0][networkIndex]);
  }, [addresses, networkIndex]);

  const value = useMemo(() => ({ tokenAddress, setTokenAddress, networkIndex }), [tokenAddress, networkIndex]);

  return <Provider value={value}>{children}</Provider>;
}

export { BridgeProvider, useBridge };
