import { NetworkEnum, Pair } from '../graphql/graphql';

const getLastDaysISOTimestamp = (daysCount: number) =>
  new Date(Date.now() - daysCount * 24 * 60 * 60 * 1000).toISOString();

const getAssetOptions = (pairs: Pair[], varaNetworkSymbol: string) => {
  const options = pairs
    .filter(({ isActive }) => isActive)
    .flatMap(({ varaToken, varaTokenSymbol, ethToken, ethTokenSymbol, tokenSupply }) => {
      // TODO: same logic as in tokens context but for pairs, probably worth to figure out a way to reuse it
      const isEthNative = tokenSupply === NetworkEnum.Ethereum && ethTokenSymbol.toLowerCase().includes('eth');
      const isVaraNative = tokenSupply === NetworkEnum.Vara && varaTokenSymbol.toLowerCase().includes('vara');

      const ethDisplaySymbol = isEthNative ? 'ETH' : ethTokenSymbol;
      const varaDisplaySymbol = isVaraNative ? varaNetworkSymbol : varaTokenSymbol;

      return [
        { label: `${varaDisplaySymbol} → ${ethDisplaySymbol}`, value: `${varaToken}.${ethToken}` as const },
        { label: `${ethDisplaySymbol} → ${varaDisplaySymbol}`, value: `${ethToken}.${varaToken}` as const },
      ];
    });

  return [{ label: 'All Assets', value: '' as const }, ...options];
};

export { getLastDaysISOTimestamp, getAssetOptions };
