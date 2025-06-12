import { HexString } from '@gear-js/api';
import { createContext, PropsWithChildren, useContext, useEffect, useMemo, useState } from 'react';

import { Token, useTokens } from '@/context';
import { useEthAccount } from '@/hooks';

import { usePairs } from '../history';

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
  const { data: pairs } = usePairs();
  const { tokens, addressToToken } = useTokens();

  const [tokenAddress, setTokenAddress] = useState<HexString>();

  useEffect(() => {
    if (!tokens) return;

    const defaultNetwork = ethAccount.address ? NETWORK.ETH : NETWORK.VARA;

    // TODO: active filter
    const defaultToken = tokens.find(
      ({ isActive, isNative, network }) => isActive && isNative && network === defaultNetwork,
    );

    setTokenAddress(defaultToken?.address);
  }, [ethAccount.address, tokens]);

  const token = tokenAddress ? addressToToken?.[tokenAddress] : undefined;
  const isVaraNetwork = token ? token.network === NETWORK.VARA : true;

  const pair = pairs?.find(({ varaToken, ethToken }) => varaToken === tokenAddress || ethToken === tokenAddress);
  const destinationTokenAddress = isVaraNetwork ? pair?.ethToken : pair?.varaToken;

  const destinationToken = destinationTokenAddress ? addressToToken?.[destinationTokenAddress] : undefined;

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
