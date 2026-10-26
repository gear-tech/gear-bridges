// Run with Node 22: node --experimental-strip-types js/frontend/src/features/faucet/isolated-api.check.mjs
// Reconstructed isolation regression from HTML VB-40, audited 7fe1b032.
import assert from 'node:assert/strict';

import { getIsolatedConfig, getIsolatedStatus, requestIsolatedBalance } from './isolated-api.ts';

const config = {
  genesis: '0xa12f631344f61644239b70f9ab53e4842e540d8b0c1e7adb9570c06a62c3af3f',
  chainId: 560048,
  assets: {
    TVARA: { amount: '100000000000000', decimals: 12 },
    USDC: { amount: '1000000000', decimals: 6, contract: '0xd4d65b1100b522cbde9664b9ba78c8add04e5b53' },
  },
  cooldownMs: 86400000,
};
const originalFetch = globalThis.fetch;
let body = config;
let calls = 0;

// Only the HTTP transport is replaced: exercise the production pin validation and finality boundary.
globalThis.fetch = async (url, options) => {
  assert(String(url).startsWith('/faucet/'));
  assert.equal(options.credentials, 'same-origin');
  assert.equal(options.mode, 'same-origin');
  assert.equal(options.redirect, 'error');
  assert.equal(options.cache, 'no-store');
  calls += 1;
  return Response.json(body);
};

try {
  assert.deepEqual(await getIsolatedConfig(), config);
  for (const invalid of [
    { ...config, chainId: 1 },
    { ...config, genesis: `0x${'00'.repeat(32)}` },
    { ...config, assets: { ...config.assets, USDC: { ...config.assets.USDC, contract: `0x${'00'.repeat(20)}` } } },
    { ...config, assets: { ...config.assets, TVARA: { ...config.assets.TVARA, amount: '100' } } },
  ]) {
    body = invalid;
    await assert.rejects(getIsolatedConfig, /approved test network/);
    const before = calls;
    await assert.rejects(() => requestIsolatedBalance('USDC', '0xrecipient', invalid));
    assert.equal(calls, before, 'Invalid network/token configuration must never reach a POST');
  }

  body = { status: 'pending', txHash: `0x${'ab'.repeat(32)}`, nextClaimAt: 86400001 };
  assert.equal((await requestIsolatedBalance('TVARA', 'recipient', config)).status, 'pending');

  body = { ...body, status: 'finalized' };
  await assert.rejects(() => requestIsolatedBalance('TVARA', 'recipient', config), /Expected a pending claim/);
  assert.equal((await getIsolatedStatus('TVARA', 'recipient')).status, 'finalized');

  body = { status: 'pending' };
  await assert.rejects(() => getIsolatedStatus('TVARA', 'recipient'), /transaction hash is missing/);
  console.log('Isolated faucet pins, write boundary and finalized-status separation passed');
} finally {
  globalThis.fetch = originalFetch;
}
