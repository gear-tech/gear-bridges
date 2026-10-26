import { HexString } from '@gear-js/api';
import { createPairHash } from 'gear-bridge-common';
import { createContext, PropsWithChildren, useContext, useMemo } from 'react';

import { usePairs } from '@/features/history';
import type { InboundProofProfile } from '@gear-js/bridge';
import { useNetworkType } from './network-type';
import { pairNativeKind } from './token-identity';
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

  getHistoryToken: ((sourceAddress: HexString | null, destinationAddress: HexString | null) => Token) | undefined;
  getActiveToken: ((address: HexString) => Token) | undefined;
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
  supply: 'Gear' | 'Ethereum';
  network: 'vara' | 'eth';
  isActive: boolean;
};

const TOKEN_PLACEHOLDER: Token = {
  address: '0x00',
  destinationAddress: '0x00',
  name: 'Token',
  symbol: 'Unit',
  displaySymbol: 'Unit',
  decimals: 0,
  isNative: false,
  supply: 'Ethereum',
  network: 'vara',
  isActive: false,
};

const deriveTokens = (pairs: Pair[], varaSymbol: string, profile?: InboundProofProfile) => {
  const addressToActiveToken: Record<HexString, Token> = {};
  const pairHashToHistoryToken: Record<string, Token> = {};

  pairs.forEach((pair) => {
    const varaAddress = pair.varaToken.toLowerCase() as HexString;
    const native = pairNativeKind(profile, pair);
    const isVaraNative = native === 'vara';

    const ethAddress = pair.ethToken.toLowerCase() as HexString;
    const isEthNative = native === 'eth';

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
      supply: pair.tokenSupply === NetworkEnum.Vara ? 'Gear' : 'Ethereum',
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
      supply: pair.tokenSupply === NetworkEnum.Vara ? 'Gear' : 'Ethereum',
    };

    if (pair.isActive) {
      addressToActiveToken[varaAddress] = varaToken;
      addressToActiveToken[ethAddress] = ethToken;
    }

    pairHashToHistoryToken[createPairHash(varaAddress, ethAddress)] = varaToken;
    pairHashToHistoryToken[createPairHash(ethAddress, varaAddress)] = ethToken;
  });

  return { addressToActiveToken, pairHashToHistoryToken };
};

function TokensProvider({ children }: PropsWithChildren) {
  const { data: pairs } = usePairs();
  const { NETWORK_PRESET } = useNetworkType();
  const profile = NETWORK_PRESET.INBOUND_PROOF_PROFILE.profile;

  const varaSymbol = useVaraSymbol();

  const { pairHashToHistoryToken, addressToActiveToken } = useMemo(
    () =>
      pairs && varaSymbol
        ? deriveTokens(pairs, varaSymbol, profile)
        : { pairHashToHistoryToken: undefined, addressToActiveToken: undefined },
    [pairs, varaSymbol, profile],
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

      getActiveToken: addressToActiveToken
        ? (address: HexString) => addressToActiveToken[address.toLowerCase() as HexString] || TOKEN_PLACEHOLDER
        : undefined,

      getHistoryToken: pairHashToHistoryToken
        ? (sourceAddress: HexString | null, destinationAddress: HexString | null) =>
            (sourceAddress && destinationAddress
              ? pairHashToHistoryToken[
                  createPairHash(
                    sourceAddress.toLowerCase() as HexString,
                    destinationAddress.toLowerCase() as HexString,
                  )
                ]
              : undefined) || TOKEN_PLACEHOLDER
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
