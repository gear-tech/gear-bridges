import type { Token } from './tokens';

type Selection = {
  api: unknown;
  client: unknown;
  NETWORK_PRESET: unknown;
  pairs: unknown;
  selectedToken: Token | undefined;
  getActiveToken: ((address: Token['address']) => Token) | undefined;
};

/** A delayed validation result cannot authorize a new profile, mapping, supply side or unavailable inventory. */
export function assertCurrentToken(start: Selection, current: Selection, token: Token): void {
  const active = current.getActiveToken?.(token.address);
  if (
    current.selectedToken?.address !== token.address ||
    current.selectedToken?.network !== token.network ||
    current.api !== start.api ||
    current.client !== start.client ||
    current.NETWORK_PRESET !== start.NETWORK_PRESET ||
    !current.pairs ||
    !active?.isActive ||
    active.address !== token.address ||
    active.destinationAddress !== token.destinationAddress ||
    active.isNative !== token.isNative ||
    active.supply !== token.supply ||
    active.decimals !== token.decimals ||
    active.network !== token.network
  ) {
    throw new Error('HOLD: token/profile changed while preparing signature');
  }
}
