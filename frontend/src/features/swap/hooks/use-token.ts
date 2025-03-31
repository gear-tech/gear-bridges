import { useMemo } from 'react';

import { useTokens } from '@/hooks';

import { NETWORK_INDEX } from '../consts';
import { getOptions } from '../utils';

function useToken(networkIndex: number, pairIndex: number) {
  const { addresses, symbols, decimals: tokenDecimals, isLoading } = useTokens();

  const { varaOptions, ethOptions } = useMemo(() => getOptions(addresses, symbols), [addresses, symbols]);
  const options = networkIndex === NETWORK_INDEX.VARA ? varaOptions : ethOptions;

  const address = addresses?.[pairIndex][networkIndex];
  const destinationAddress = addresses?.[pairIndex][Number(!networkIndex)];
  const symbol = address ? symbols?.[address] : undefined;
  const destinationSymbol = destinationAddress ? symbols?.[destinationAddress] : undefined;
  const decimals = address ? tokenDecimals?.[address] : undefined;

  return { address, destinationAddress, destinationSymbol, options, symbol, decimals, isLoading };
}

export { useToken };
