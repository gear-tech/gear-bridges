import { createContext, PropsWithChildren, useContext, useMemo, useState } from 'react';

import { NETWORK_INDEX } from './consts';

type Value = {
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
} as const;

const BridgeContext = createContext<Value>(DEFAULT_VALUE);
const { Provider } = BridgeContext;
const useBridgeContext = () => useContext(BridgeContext);

function BridgeProvider({ children }: PropsWithChildren) {
  const [networkIndex, setNetworkIndex] = useState(NETWORK_INDEX.VARA);
  const isVaraNetwork = networkIndex === NETWORK_INDEX.VARA;

  const [pairIndex, setPairIndex] = useState(0);

  const switchNetwork = () => setNetworkIndex((prevValue) => Number(!prevValue));

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
    }),
    [networkIndex, isVaraNetwork, pairIndex],
  );

  return <Provider value={value}>{children}</Provider>;
}

// eslint-disable-next-line react-refresh/only-export-components
export { BridgeProvider, useBridgeContext };
