import { HexString } from '@gear-js/api';

import { usePairs } from '@/features/history';
import { Network } from '@/features/history/graphql/graphql';

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
    symbols: undefined,
    decimals: undefined,
  };

  const varaPair = activePairs?.find(
    (pair) => pair.varaTokenSymbol.toLowerCase().includes('vara') && pair.tokenSupply === Network.Gear,
  );

  const ethPair = activePairs?.find(
    (pair) => pair.ethTokenSymbol.toLowerCase().includes('eth') && pair.tokenSupply === Network.Ethereum,
  );

  const wrappedVaraAddress = varaPair?.varaToken;
  const wrappedEthAddress = ethPair?.ethToken;

  return { addresses, symbols, decimals, isLoading, wrappedVaraAddress, wrappedEthAddress };
}

export { useTokens };
