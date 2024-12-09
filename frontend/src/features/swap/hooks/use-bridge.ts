import { useMemo, useState } from 'react';

import { useTokens } from '@/hooks';

import { NETWORK_INDEX } from '../consts';
import { getOptions } from '../utils';

function useBridge(networkIndex: number) {
  const isVaraNetwork = networkIndex === NETWORK_INDEX.VARA;

  const { addresses, symbols, decimals: tokenDecimals, isLoading } = useTokens();

  const { varaOptions, ethOptions } = useMemo(() => getOptions(addresses, symbols), [addresses, symbols]);
  const options = { from: isVaraNetwork ? varaOptions : ethOptions, to: isVaraNetwork ? ethOptions : varaOptions };

  const [pair, setPair] = useState('0');
  const pairIndex = Number(pair);
  const address = addresses?.[pairIndex][networkIndex];
  const destinationAddress =
    addresses?.[pairIndex][networkIndex === NETWORK_INDEX.VARA ? NETWORK_INDEX.ETH : NETWORK_INDEX.VARA];
  const symbol = address ? symbols?.[address] : undefined;
  const decimals = address ? tokenDecimals?.[address] : undefined;

  return { address, destinationAddress, options, symbol, decimals, pair: { value: pair, set: setPair }, isLoading };
}

export { useBridge };
