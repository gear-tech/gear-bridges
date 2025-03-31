import { createContext, PropsWithChildren, useContext, useMemo, useState } from 'react';

import { NETWORK_INDEX } from './consts';

type Value = {
  network: {
    index: number;
    isVara: boolean;
    switch: () => void;
  };
};

const DEFAULT_VALUE = {
  network: {
    index: NETWORK_INDEX.VARA,
    isVara: true,
    switch: () => {},
  },
} as const;

const BridgeContext = createContext<Value>(DEFAULT_VALUE);
const { Provider } = BridgeContext;
const useBridgeContext = () => useContext(BridgeContext);

function BridgeProvider({ children }: PropsWithChildren) {
  const [networkIndex, setNetworkIndex] = useState(NETWORK_INDEX.VARA);
  const isVaraNetwork = networkIndex === NETWORK_INDEX.VARA;

  const switchNetwork = () => setNetworkIndex((prevValue) => Number(!prevValue));

  const value = useMemo(
    () => ({
      network: {
        index: networkIndex,
        isVara: isVaraNetwork,
        switch: switchNetwork,
      },
    }),
    [networkIndex, isVaraNetwork],
  );

  return <Provider value={value}>{children}</Provider>;
}

// eslint-disable-next-line react-refresh/only-export-components
export { BridgeProvider, useBridgeContext };
