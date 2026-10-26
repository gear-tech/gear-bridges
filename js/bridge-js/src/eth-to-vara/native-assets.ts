import type { InboundProofProfile } from './proof-composer.js';

export function validateTokenPairInventory(pairs: readonly { varaToken: string; ethToken: string; tokenSupply: string; isActive: boolean }[], mappings: readonly [string, string, string][]): void {
  const seen = new Set<string>();
  const active = pairs.filter(pair => pair.isActive);
  for (const pair of active) {
    const vara = pair.varaToken.toLowerCase(), eth = pair.ethToken.toLowerCase();
    if (!/^0x[0-9a-f]{64}$/.test(vara) || !/^0x[0-9a-f]{40}$/.test(eth) || BigInt(vara) === 0n || BigInt(eth) === 0n || seen.has(vara) || seen.has(eth) || !['Gear', 'Ethereum'].includes(pair.tokenSupply)) throw new Error('HOLD: conflicting or malformed active token pair');
    seen.add(vara); seen.add(eth);
    if (mappings.filter(([v, e, s]) => v.toLowerCase() === vara && e.toLowerCase() === eth && s === pair.tokenSupply).length !== 1 ||
        mappings.some(([v, e]) => (v.toLowerCase() === vara || e.toLowerCase() === eth) && (v.toLowerCase() !== vara || e.toLowerCase() !== eth))) throw new Error('HOLD: indexer token pair differs from finalized manager mapping');
  }
  if (active.length !== mappings.length) throw new Error('HOLD: active indexer inventory is incomplete');
}

/** Metadata never grants a native-value capability. Program/code bindings are checked at the finalized pin. */
export function nativeAssetKind(profile: InboundProofProfile | undefined, varaToken: string, ethereumToken: string,
  supply: 'Gear' | 'Ethereum'): 'vara' | 'eth' | undefined {
  const vara = varaToken.toLowerCase(), eth = ethereumToken.toLowerCase();
  if (supply === 'Gear' && profile?.nativeWrapper && vara === profile.nativeWrapper.programId.toLowerCase() &&
      eth === profile.nativeWrapper.ethereumToken.toLowerCase()) return 'vara';
  if (supply === 'Ethereum' && profile?.nativeEth && vara === profile.nativeEth.varaToken.toLowerCase() &&
      eth === profile.nativeEth.ethereumToken.toLowerCase()) return 'eth';
  return undefined;
}
