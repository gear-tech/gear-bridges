import { HexString } from '@gear-js/api';
import { createContext, PropsWithChildren, useContext, useMemo } from 'react';

import { usePairs } from '@/features/history';
import { Network, Pair } from '@/features/history/graphql/graphql';

type Value = {
  addressToToken: Record<HexString, Token> | undefined;
  tokens: Token[] | undefined;
};

const DEFAULT_VALUE = {
  addressToToken: undefined,
  tokens: undefined,
};

const TokensContext = createContext<Value>(DEFAULT_VALUE);
const { Provider } = TokensContext;
const useTokens = () => useContext(TokensContext);

type Token = {
  address: HexString;
  symbol: string;
  decimals: number;
  isNative: boolean;
  network: 'vara' | 'eth';
  isActive: boolean;
};

const deriveTokens = (pairs: Pair[]) => {
  const addressToToken: Record<HexString, Token> = {};

  pairs.forEach((pair) => {
    const varaAddress = pair.varaToken as HexString;
    const ethAddress = pair.ethToken as HexString;

    const varaToken: Token = {
      address: varaAddress,
      symbol: pair.varaTokenSymbol,
      decimals: pair.varaTokenDecimals,
      isNative: pair.tokenSupply === Network.Gear && pair.varaTokenSymbol.toLowerCase().includes('vara'),
      network: 'vara',
      isActive: !pair.isRemoved,
    };

    const ethToken: Token = {
      address: ethAddress,
      symbol: pair.ethTokenSymbol,
      decimals: pair.ethTokenDecimals,
      isNative: pair.tokenSupply === Network.Ethereum && pair.ethTokenSymbol.toLowerCase().includes('eth'),
      network: 'eth',
      isActive: !pair.isRemoved,
    };

    addressToToken[varaAddress] = varaToken;
    addressToToken[ethAddress] = ethToken;
  });

  return addressToToken;
};

function TokensProvider({ children }: PropsWithChildren) {
  const { data: pairs } = usePairs();

  const addressToToken = useMemo(() => (pairs ? deriveTokens(pairs) : undefined), [pairs]);
  const tokens = useMemo(() => (addressToToken ? Object.values(addressToToken) : undefined), [addressToToken]);
  const value = useMemo(() => ({ addressToToken, tokens }), [addressToToken, tokens]);

  return <Provider value={value}>{children}</Provider>;
}

// eslint-disable-next-line react-refresh/only-export-components
export { TokensProvider, useTokens };
export type { Token };
