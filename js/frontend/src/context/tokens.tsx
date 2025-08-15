import { HexString } from '@gear-js/api';
import { getPairHash } from '@workspace/common';
import { createContext, PropsWithChildren, useContext, useMemo } from 'react';

import { usePairs } from '@/features/history';
import { NetworkEnum, Pair } from '@/features/history/graphql/graphql';
import { NETWORK } from '@/features/swap/consts';
import { useVaraSymbol } from '@/hooks';

type Value = {
  pairHashToToken: Record<string, Token> | undefined;

  tokens: {
    active: Token[] | undefined;
    vara: Token[] | undefined;
    eth: Token[] | undefined;
  };

  nativeToken: {
    vara: Token | undefined;
    eth: Token | undefined;
  };
};

const DEFAULT_VALUE = {
  pairHashToToken: undefined,

  tokens: {
    active: undefined,
    vara: undefined,
    eth: undefined,
  },

  nativeToken: {
    vara: undefined,
    eth: undefined,
  },
};

const TokensContext = createContext<Value>(DEFAULT_VALUE);
const { Provider } = TokensContext;
const useTokens = () => useContext(TokensContext);

type Token = {
  address: HexString;
  destinationAddress: HexString;
  name: string;
  symbol: string;
  displaySymbol: string;
  decimals: number;
  isNative: boolean;
  network: 'vara' | 'eth';
  isActive: boolean;
};

const deriveTokens = (pairs: Pair[], varaSymbol: string) => {
  const pairHashToToken: Record<string, Token> = {};

  pairs.forEach((pair) => {
    const varaAddress = pair.varaToken as HexString;
    const isVaraNative = pair.tokenSupply === NetworkEnum.Vara && pair.varaTokenSymbol.toLowerCase().includes('vara');

    const ethAddress = pair.ethToken as HexString;
    const isEthNative = pair.tokenSupply === NetworkEnum.Ethereum && pair.ethTokenSymbol.toLowerCase().includes('eth');

    // changing wrapped native symbol to native symbol
    const varaDisplaySymbol = isVaraNative ? varaSymbol : pair.varaTokenSymbol;
    const ethDisplaySymbol = isEthNative ? 'ETH' : pair.ethTokenSymbol;

    const varaToken: Token = {
      address: varaAddress,
      destinationAddress: ethAddress,
      name: pair.varaTokenName,
      symbol: pair.varaTokenSymbol,
      displaySymbol: varaDisplaySymbol,
      decimals: pair.varaTokenDecimals,
      isNative: isVaraNative,
      network: 'vara',
      isActive: pair.isActive,
    };

    const ethToken: Token = {
      address: ethAddress,
      destinationAddress: varaAddress,
      name: pair.ethTokenName,
      symbol: pair.ethTokenSymbol,
      displaySymbol: ethDisplaySymbol,
      decimals: pair.ethTokenDecimals,
      isNative: isEthNative,
      network: 'eth',
      isActive: pair.isActive,
    };

    pairHashToToken[getPairHash(varaAddress, ethAddress)] = varaToken;
    pairHashToToken[getPairHash(ethAddress, varaAddress)] = ethToken;
  });

  return pairHashToToken;
};

function TokensProvider({ children }: PropsWithChildren) {
  const { data: pairs } = usePairs();
  const varaSymbol = useVaraSymbol();

  const pairHashToToken = useMemo(
    () => (pairs && varaSymbol ? deriveTokens(pairs, varaSymbol) : undefined),
    [pairs, varaSymbol],
  );

  console.log('pairHashToToken: ', pairHashToToken);

  const tokens = useMemo(() => (pairHashToToken ? Object.values(pairHashToToken) : undefined), [pairHashToToken]);

  const activeTokens = useMemo(() => tokens?.filter(({ isActive }) => isActive), [tokens]);
  const varaTokens = useMemo(() => activeTokens?.filter(({ network }) => network === NETWORK.VARA), [activeTokens]);
  const ethTokens = useMemo(() => activeTokens?.filter(({ network }) => network === NETWORK.ETH), [activeTokens]);

  const nativeVaraToken = useMemo(() => varaTokens?.find(({ isNative }) => isNative), [varaTokens]);
  const nativeEthToken = useMemo(() => ethTokens?.find(({ isNative }) => isNative), [ethTokens]);

  const value = useMemo(
    () => ({
      pairHashToToken,

      tokens: {
        active: activeTokens,
        vara: varaTokens,
        eth: ethTokens,
      },

      nativeToken: {
        vara: nativeVaraToken,
        eth: nativeEthToken,
      },
    }),
    [pairHashToToken, activeTokens, varaTokens, ethTokens, nativeVaraToken, nativeEthToken],
  );

  return <Provider value={value}>{children}</Provider>;
}

// eslint-disable-next-line react-refresh/only-export-components
export { TokensProvider, useTokens };
export type { Token };
