import { HexString } from '@gear-js/api';
import { useMemo, useState } from 'react';

import { NETWORK_INDEX } from '../consts';
import { getOptions } from '../utils';

import { useFTSymbols } from './use-ft-symbols';
import { useFTAddresses } from './vara';

function useBridge(networkIndex: number) {
  const isVaraNetwork = networkIndex === NETWORK_INDEX.VARA;
  const nativeSymbol = isVaraNetwork ? 'VARA' : 'ETH';

  const { data: ftAddresses } = useFTAddresses();
  const { data: ftSymbols, isPending } = useFTSymbols(ftAddresses);

  const { varaOptions, ethOptions } = useMemo(() => getOptions(ftSymbols), [ftSymbols]);
  const options = { from: isVaraNetwork ? varaOptions : ethOptions, to: isVaraNetwork ? ethOptions : varaOptions };

  const [pair, setPair] = useState('0');
  const pairIndex = Number(pair);
  const address = ftAddresses?.[pairIndex][networkIndex].toString() as HexString | undefined;
  const symbol = ftSymbols?.[pairIndex][networkIndex];

  return { address, options, symbol, nativeSymbol, pair: { value: pair, set: setPair }, isLoading: isPending };
}

export { useBridge };
