import { useMemo, useState } from 'react';

import { NETWORK_NAME, NETWORK_NATIVE_SYMBOL, SPEC } from '@/consts';

import { METADATA_URL } from '../consts';
import { NetworkName } from '../types';
import { getOptions } from '../utils';

import { useMetadata } from './vara';

function useBridge(networkName: NetworkName) {
  const isVaraNetwork = networkName === NETWORK_NAME.VARA;

  const { varaOptions, ethOptions } = useMemo(getOptions, []);
  const options = { from: isVaraNetwork ? varaOptions : ethOptions, to: isVaraNetwork ? ethOptions : varaOptions };

  const [pair, setPair] = useState(options.from[0].value);
  const bridge = SPEC[pair][networkName];
  const { address } = bridge;

  const metadata = useMetadata(isVaraNetwork ? METADATA_URL[bridge.tokenType] : undefined);
  const contract = { address, metadata };

  const nativeSymbol = NETWORK_NATIVE_SYMBOL[networkName];
  const symbol = { value: bridge.symbol, native: nativeSymbol };

  return { contract, options, symbol, pair: { value: pair, set: setPair } };
}

export { useBridge };
