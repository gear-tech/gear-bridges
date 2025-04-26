import { HexString } from '@gear-js/api';
import { createContext, PropsWithChildren, useContext, useMemo, useState } from 'react';

import { useTokens } from '@/hooks';
import { isNativeToken } from '@/utils';

import { NETWORK_INDEX } from './consts';

type Context = {
  network: {
    index: number;
    isVara: boolean;
    setIndex: (index: number) => void;
    switch: () => void;
  };

  pair: {
    index: number;
    setIndex: (index: number) => void;
  };

  token: {
    address: HexString | undefined;
    symbol: string | undefined;
    decimals: number | undefined;
    isNative: boolean;

    destination: {
      address: HexString | undefined;
      symbol: string | undefined;
    };
  };
};

const DEFAULT_VALUE = {
  network: {
    index: NETWORK_INDEX.VARA,
    isVara: true,
    setIndex: () => {},
    switch: () => {},
  },

  pair: {
    index: 0,
    setIndex: () => {},
  },

  token: {
    address: undefined,
    symbol: undefined,
    decimals: undefined,
    isNative: false,

    destination: {
      address: undefined,
      symbol: undefined,
    },
  },
} as const;

const BridgeContext = createContext<Context>(DEFAULT_VALUE);
const { Provider } = BridgeContext;
const useBridgeContext = () => useContext(BridgeContext);

function BridgeProvider({ children }: PropsWithChildren) {
  // network
  const [networkIndex, setNetworkIndex] = useState(NETWORK_INDEX.VARA);
  const isVaraNetwork = networkIndex === NETWORK_INDEX.VARA;

  const switchNetwork = () => setNetworkIndex((prevValue) => Number(!prevValue));

  // pair
  const [pairIndex, setPairIndex] = useState(0);

  // token
  const { addresses, symbols, decimals } = useTokens();
  const tokenAddress = addresses?.[pairIndex][networkIndex];
  const tokenSymbol = tokenAddress ? symbols?.[tokenAddress] : undefined;
  const tokenDecimals = tokenAddress ? decimals?.[tokenAddress] : undefined;
  const tokenDestinationAddress = addresses?.[pairIndex][Number(!networkIndex)];
  const tokenDestinationSymbol = tokenDestinationAddress ? symbols?.[tokenDestinationAddress] : undefined;

  const value = useMemo(
    () => ({
      network: {
        index: networkIndex,
        isVara: isVaraNetwork,
        setIndex: setNetworkIndex,
        switch: switchNetwork,
      },

      pair: {
        index: pairIndex,
        setIndex: setPairIndex,
      },

      token: {
        address: tokenAddress,
        symbol: tokenSymbol,
        decimals: tokenDecimals,
        isNative: tokenAddress ? isNativeToken(tokenAddress) : false,

        destination: {
          address: tokenDestinationAddress,
          symbol: tokenDestinationSymbol,
        },
      },
    }),
    [
      networkIndex,
      isVaraNetwork,
      pairIndex,
      tokenAddress,
      tokenSymbol,
      tokenDecimals,
      tokenDestinationAddress,
      tokenDestinationSymbol,
    ],
  );

  return <Provider value={value}>{children}</Provider>;
}

// eslint-disable-next-line react-refresh/only-export-components
export { BridgeProvider, useBridgeContext };
