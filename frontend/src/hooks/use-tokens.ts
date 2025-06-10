import { HexString } from '@gear-js/api';

import { usePairs } from '@/api';

function useTokens() {
  const { data: pairs, isLoading } = usePairs();
  const activePairs = pairs?.filter(({ isRemoved }) => !isRemoved);

  const { addresses, symbols, decimals } = activePairs?.reduce(
    (result, pair) => {
      const { varaToken, varaTokenSymbol, varaTokenDecimals, ethToken, ethTokenSymbol, ethTokenDecimals } = pair;

      result.addresses.push([varaToken, ethToken]);

      result.symbols[varaToken] = varaTokenSymbol;
      result.decimals[varaToken] = varaTokenDecimals;

      result.symbols[ethToken] = ethTokenSymbol;
      result.decimals[ethToken] = ethTokenDecimals;

      return result;
    },
    {
      addresses: [] as [HexString, HexString][],
      symbols: {} as Record<HexString, string>,
      decimals: {} as Record<HexString, number>,
    },
  ) || {
    addresses: undefined,
    symbols: {} as Record<HexString, string>,
    decimals: {} as Record<HexString, number>,
  };

  return { addresses, symbols, decimals, isLoading };
}

export { useTokens };
