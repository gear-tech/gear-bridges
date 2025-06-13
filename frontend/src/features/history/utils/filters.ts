import { Pair } from '../graphql/graphql';

const getLastDaysISOTimestamp = (daysCount: number) =>
  new Date(Date.now() - daysCount * 24 * 60 * 60 * 1000).toISOString();

const getAssetOptions = (pairs: Pair[]) => {
  const options = pairs
    .filter(({ isRemoved }) => !isRemoved)
    .flatMap(({ varaToken, varaTokenSymbol, ethToken, ethTokenSymbol }) => [
      { label: `${varaTokenSymbol} → ${ethTokenSymbol}`, value: `${varaToken}.${ethToken}` as const },
      { label: `${ethTokenSymbol} → ${varaTokenSymbol}`, value: `${ethToken}.${varaToken}` as const },
    ]);

  return [{ label: 'All Assets', value: '' as const }, ...options];
};

export { getLastDaysISOTimestamp, getAssetOptions };
