import { describe, expect, test } from 'vitest';
import { relayFailure } from '../example/relay-failure.js';

describe('SDK application example reconciliation reporting', () => {
  const ethereumTransactionHash = '0x' + 'ab'.repeat(32);
  const gearTransactionHash = '0x' + 'cd'.repeat(32);
  const sensitive = 'https://user:secret@provider.invalid/private-token?apiKey=secret //SecretSigner';

  test('retains the unsuccessful original outbound hash, nonce and sanitized reason', () => {
    const result = { success: false, transactionHash: ethereumTransactionHash,
      error: 'Original relay deadline expired: ' + sensitive };
    const report = relayFailure(new Error(result.error), {
      ethereumTransactionHash: result.transactionHash, nonce: '0', sourceBlockNumber: '42',
    });
    expect(report).toMatchObject({ ethereumTransactionHash, nonce: '0', sourceBlockNumber: '42',
      reason: 'Original deadline expired; reconcile any admitted request without extending it.' });
    expect(JSON.stringify(report)).not.toContain('secret');
    expect(JSON.stringify(report)).not.toContain('provider.invalid');
  });

  test('retains original inbound Ethereum and wrapped Gear IDs without logging arbitrary errors', () => {
    const report = relayFailure(new Error(`HOLD: original Gear transaction ${gearTransactionHash}: InsufficientBalance: ${sensitive}. Reconcile the original before retrying.`),
      { ethereumTransactionHash });
    expect(report).toMatchObject({ ethereumTransactionHash, gearTransactionHash,
      reason: 'Original Gear dispatch failed for insufficient balance; reconcile its transaction before retrying.' });
    expect(JSON.stringify(report)).not.toContain('secret');
    expect(JSON.stringify(report)).not.toContain('provider.invalid');
    expect(relayFailure(new Error(sensitive), { ethereumTransactionHash, gearTransactionHash })).toMatchObject({
      ethereumTransactionHash, gearTransactionHash,
      reason: 'Original delivery is unproven; reconcile the original chain and signer/account history.',
    });
  });

  test('never invents an unknown wallet hash or echoes malformed identity input', () => {
    const report = relayFailure('HOLD: original wallet submission outcome unresolved; ' + sensitive,
      { ethereumTransactionHash: '0x', gearTransactionHash: sensitive, nonce: sensitive, sourceBlockNumber: sensitive });
    expect(report).toEqual({ ethereumTransactionHash: undefined, gearTransactionHash: undefined, nonce: undefined,
      sourceBlockNumber: undefined, reason: 'Original wallet submission may still broadcast; reconcile its account/nonce.' });
  });
});
