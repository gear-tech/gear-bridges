import { nativeAssetKind, type InboundProofProfile } from '@gear-js/bridge';
import { NetworkEnum, Pair } from '../graphql/graphql';

const getLastDaysISOTimestamp = (daysCount: number) =>
  new Date(Date.now() - daysCount * 24 * 60 * 60 * 1000).toISOString();

const getAssetOptions = (pairs: Pair[], varaNetworkSymbol: string, profile?: InboundProofProfile) => {
  const options = pairs
    .filter(({ isActive }) => isActive)
    .flatMap(({ varaToken, varaTokenSymbol, ethToken, ethTokenSymbol, tokenSupply }) => {
      const kind = nativeAssetKind(
        profile,
        varaToken,
        ethToken,
        tokenSupply === NetworkEnum.Vara ? 'Gear' : 'Ethereum',
      );
      if (kind === 'vara') varaTokenSymbol = varaNetworkSymbol;
      if (kind === 'eth') ethTokenSymbol = 'ETH';
      return [
        { label: `${varaTokenSymbol} → ${ethTokenSymbol}`, value: `${varaToken}.${ethToken}` as const },
        { label: `${ethTokenSymbol} → ${varaTokenSymbol}`, value: `${ethToken}.${varaToken}` as const },
      ];
    });

  return [{ label: 'All Assets', value: '' as const }, ...options];
};

export { getLastDaysISOTimestamp, getAssetOptions };
