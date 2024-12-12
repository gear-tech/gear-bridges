import { useMemo } from 'react';

import { useBridge } from '@/contexts';
import { useTokens } from '@/hooks';

import { NETWORK_INDEX } from '../consts';
import { getOptions } from '../utils';

function useToken() {
  const { tokenAddress, networkIndex } = useBridge();
  const { addresses, symbols, decimals: tokenDecimals, isLoading } = useTokens();

  const { varaOptions, ethOptions } = useMemo(() => getOptions(addresses, symbols), [addresses, symbols]);
  const options = networkIndex === NETWORK_INDEX.VARA ? varaOptions : ethOptions;

  const destinationAddress = addresses?.find((pair) => pair[networkIndex] === tokenAddress)?.[Number(!networkIndex)];

  const symbol = tokenAddress ? symbols?.[tokenAddress] : undefined;
  const destinationSymbol = destinationAddress ? symbols?.[destinationAddress] : undefined;
  const decimals = tokenAddress ? tokenDecimals?.[tokenAddress] : undefined;

  return { destinationAddress, destinationSymbol, options, symbol, decimals, isLoading };
}

export { useToken };
