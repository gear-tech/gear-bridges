import { HexString } from '@gear-js/api';

import { FTAddressPair } from '@/types';

const getLastDaysISOTimestamp = (daysCount: number) =>
  new Date(Date.now() - daysCount * 24 * 60 * 60 * 1000).toISOString();

const getAssetOptions = (addresses: FTAddressPair[], symbols: Record<HexString, string>) => {
  const options = [] as { label: string; value: string }[];

  for (const [varaAddress, ethAddress] of addresses) {
    const varaSymbol = symbols[varaAddress];
    const ethSymbol = symbols[ethAddress];

    options.push({
      label: `${varaSymbol} → ${ethSymbol}`,
      value: `${varaAddress}.${ethAddress}` as const,
    });

    options.push({
      label: `${ethSymbol} → ${varaSymbol}`,
      value: `${ethAddress}.${varaAddress}` as const,
    });
  }

  return [{ label: 'All Assets', value: '' as const }, ...options];
};

export { getLastDaysISOTimestamp, getAssetOptions };
