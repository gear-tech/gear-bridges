import { HexString } from '@gear-js/api';
import { getPairHash } from '@workspace/common';
import { createContext, PropsWithChildren, useContext, useMemo } from 'react';

import { usePairs } from '@/features/history';
import { NetworkEnum, Pair } from '@/features/history/graphql/graphql';
import { NETWORK } from '@/features/swap/consts';
import { useVaraSymbol } from '@/hooks';

type Value = {
  tokens: {
    active: Token[] | undefined;
    vara: Token[] | undefined;
    eth: Token[] | undefined;
  };

  nativeToken: {
    vara: Token | undefined;
    eth: Token | undefined;
  };

  getHistoryToken: ((sourceAddress: HexString, destinationAddress: HexString) => Token | undefined) | undefined;
  getActiveToken: ((address: HexString) => Token | undefined) | undefined;
};

const DEFAULT_VALUE = {
  tokens: {
    active: undefined,
    vara: undefined,
    eth: undefined,
  },

  nativeToken: {
    vara: undefined,
    eth: undefined,
  },

  getActiveToken: undefined,
  getHistoryToken: undefined,
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
  const addressToActiveToken: Record<HexString, Token> = {};
  const pairHashToHistoryToken: Record<string, Token> = {};

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

    if (pair.isActive) {
      addressToActiveToken[varaAddress] = varaToken;
      addressToActiveToken[ethAddress] = ethToken;
    }

    pairHashToHistoryToken[getPairHash(varaAddress, ethAddress)] = varaToken;
    pairHashToHistoryToken[getPairHash(ethAddress, varaAddress)] = ethToken;
  });

  return { addressToActiveToken, pairHashToHistoryToken };
};

function TokensProvider({ children }: PropsWithChildren) {
  const { data: pairs } = usePairs();

  const varaSymbol = useVaraSymbol();

  const { pairHashToHistoryToken, addressToActiveToken } = useMemo(
    () =>
      pairs && varaSymbol
        ? deriveTokens(pairs, varaSymbol)
        : { pairHashToHistoryToken: undefined, addressToActiveToken: undefined },
    [pairs, varaSymbol],
  );

  const activeTokens = useMemo(
    () => (addressToActiveToken ? Object.values(addressToActiveToken) : undefined),
    [addressToActiveToken],
  );

  const varaTokens = useMemo(() => activeTokens?.filter(({ network }) => network === NETWORK.VARA), [activeTokens]);
  const ethTokens = useMemo(() => activeTokens?.filter(({ network }) => network === NETWORK.ETH), [activeTokens]);

  const nativeVaraToken = useMemo(() => varaTokens?.find(({ isNative }) => isNative), [varaTokens]);
  const nativeEthToken = useMemo(() => ethTokens?.find(({ isNative }) => isNative), [ethTokens]);

  const value = useMemo(
    () => ({
      tokens: {
        active: activeTokens,
        vara: varaTokens,
        eth: ethTokens,
      },

      nativeToken: {
        vara: nativeVaraToken,
        eth: nativeEthToken,
      },

      getActiveToken: addressToActiveToken ? (address: HexString) => addressToActiveToken[address] : undefined,

      getHistoryToken: pairHashToHistoryToken
        ? (sourceAddress: HexString, destinationAddress: HexString) =>
            pairHashToHistoryToken[getPairHash(sourceAddress, destinationAddress)]
        : undefined,
    }),
    [
      activeTokens,
      varaTokens,
      ethTokens,
      nativeVaraToken,
      nativeEthToken,
      addressToActiveToken,
      pairHashToHistoryToken,
    ],
  );

  return <Provider value={value}>{children}</Provider>;
}

// eslint-disable-next-line react-refresh/only-export-components
export { TokensProvider, useTokens };
export type { Token };
