type OriginalRelayIdentity = {
  ethereumTransactionHash?: string;
  gearTransactionHash?: string;
  nonce?: string;
  sourceBlockNumber?: string;
};

/** Report only original public identities and fixed reasons, never arbitrary RPC/error text. */
export function relayFailure(error: unknown, original: OriginalRelayIdentity) {
  const message = error instanceof Error ? error.message : typeof error === 'string' ? error : '';
  const gearHash = /^HOLD: original Gear transaction (0x[0-9a-fA-F]{64}):/.exec(message)?.[1];
  const reasons = [
    ['deadline expired', 'Original deadline expired; reconcile any admitted request without extending it.'],
    ['original wallet submission outcome unresolved', 'Original wallet submission may still broadcast; reconcile its account/nonce.'],
    ['InsufficientBalance', 'Original Gear dispatch failed for insufficient balance; reconcile its transaction before retrying.'],
    ['Original finalized processMessage transaction reverted', 'Original finalized Ethereum queue transaction reverted.'],
    ['Finalized proxy error:', 'Original finalized proxy reply rejected the proof.'],
    ['Finalized consumer error', 'Original finalized consumer reply did not establish delivery.'],
    ['original admitted Gear request needs reconciliation', 'Original Gear admission is unresolved.'],
    ['original Gear request inclusion has no unambiguous queued identity', 'Original Gear inclusion has no authenticated queued identity.'],
    ['effect', 'Original finalized application effect could not be authenticated.'],
    ['payload', 'Original application payload could not be authenticated.'],
  ];
  const ethereumTransactionHash = original.ethereumTransactionHash;
  const gearTransactionHash = gearHash ?? original.gearTransactionHash;
  return {
    ethereumTransactionHash: ethereumTransactionHash && /^0x[0-9a-fA-F]{64}$/.test(ethereumTransactionHash)
      ? ethereumTransactionHash : undefined,
    gearTransactionHash: gearTransactionHash && /^0x[0-9a-fA-F]{64}$/.test(gearTransactionHash)
      ? gearTransactionHash : undefined,
    nonce: original.nonce && /^[0-9]+$/.test(original.nonce) ? original.nonce : undefined,
    sourceBlockNumber: original.sourceBlockNumber && /^[0-9]+$/.test(original.sourceBlockNumber)
      ? original.sourceBlockNumber : undefined,
    reason: reasons.find(([fragment]) => message.includes(fragment))?.[1] ??
      'Original delivery is unproven; reconcile the original chain and signer/account history.',
  };
}
