import { HexString } from '@gear-js/api';
import { createContext, PropsWithChildren, useContext, useEffect, useMemo, useState } from 'react';

import { Token, useTokens } from '@/context';
import { useEthAccount } from '@/hooks';

import { NETWORK } from './consts';

type Context = {
  network: {
    name: 'vara' | 'eth';
    isVara: boolean;
    switch: () => void;
  };

  token: (Token & { set: (address: HexString) => void }) | undefined;
  destinationToken: Token | undefined;
};

const DEFAULT_VALUE = {
  network: {
    name: NETWORK.VARA,
    isVara: true,
    switch: () => {},
  },

  token: undefined,
  destinationToken: undefined,
} as const;

const BridgeContext = createContext<Context>(DEFAULT_VALUE);
const { Provider } = BridgeContext;
const useBridgeContext = () => useContext(BridgeContext);

function BridgeProvider({ children }: PropsWithChildren) {
  // network
  const ethAccount = useEthAccount();

  // token
  const { getActiveToken, nativeToken } = useTokens();

  const defaultNetwork = ethAccount.address ? NETWORK.ETH : NETWORK.VARA;
  const defaultTokenAddress = nativeToken[defaultNetwork]?.address;
  const [tokenAddress, setTokenAddress] = useState(defaultTokenAddress);

  useEffect(() => {
    setTokenAddress(defaultTokenAddress);
  }, [defaultTokenAddress]);

  const token = tokenAddress ? getActiveToken?.(tokenAddress) : undefined;
  const destinationToken = token?.destinationAddress ? getActiveToken?.(token.destinationAddress) : undefined;
  const isVaraNetwork = token ? token.network === NETWORK.VARA : true;

  const value = useMemo(
    () => ({
      network: {
        name: token?.network || NETWORK.VARA,
        isVara: isVaraNetwork,
        switch: () => setTokenAddress(destinationToken?.address),
      },

      token: token ? { ...token, set: setTokenAddress } : undefined,
      destinationToken,
    }),
    [destinationToken, isVaraNetwork, token],
  );

  return <Provider value={value}>{children}</Provider>;
}

// eslint-disable-next-line react-refresh/only-export-components
export { BridgeProvider, useBridgeContext };
