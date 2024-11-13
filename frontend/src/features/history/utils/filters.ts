import { HexString } from '@gear-js/api';
import { ActorId, H160 } from 'sails-js';

const getLastDaysISOTimestamp = (daysCount: number) =>
  new Date(Date.now() - daysCount * 24 * 60 * 60 * 1000).toISOString();

const getAssetOptions = (addresses: [ActorId, H160][], symbols: Record<HexString, string>) => {
  const options = [] as { label: string; value: string }[];

  for (const pair of addresses) {
    const varaAddress = pair[0].toString() as HexString;
    const ethAddress = pair[1].toString() as HexString;

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
