'use strict';

// Offline only: real signatures, authenticated filesystem journal, concurrency,
// restart and injected transport loss. No network, funding or broadcasts.
const test = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const { createHash } = require('node:crypto');
const { Wallet, Transaction, keccak256, getAddress } = require('ethers');
const { encodeAddress, cryptoWaitReady } = require('@polkadot/util-crypto');
const { Journal, Faucet, admission, normalize, AMOUNTS, COOLDOWN, TOKEN, CHAIN_ID, ABI } = require('./test-faucet.cjs');

test('aliases, daily limits, concurrent originals, restart HOLD and journal tampering', async () => {
  await cryptoWaitReady();
  const bytes = Uint8Array.from({ length: 32 }, (_, i) => i + 1);
  const alias42 = encodeAddress(bytes, 42), alias137 = encodeAddress(bytes, 137);
  assert.equal(normalize('TVARA', alias42), normalize('TVARA', alias137));
  const now = 1800000000000;
  const prior = { asset: 'TVARA', address: normalize('TVARA', alias42), amount: AMOUNTS.TVARA, status: 'finalized', nextClaimAt: now + COOLDOWN };
  assert.throws(() => admission([prior], 'TVARA', alias137, now), e => e.code === 429 && e.nextClaimAt === prior.nextClaimAt);
  assert.equal(admission([prior], 'TVARA', alias137, prior.nextClaimAt), null);
  assert.equal(admission([{ ...prior, status: 'failed' }], 'TVARA', alias137, prior.nextClaimAt), null);
  assert.throws(() => admission([{ ...prior, status: 'failed' }], 'TVARA', alias137, now), e => e.code === 429);
  const pending = { ...prior, status: 'pending' };
  assert.equal(admission([pending], 'TVARA', alias137, now), pending);
  assert.throws(() => admission([pending], 'TVARA', encodeAddress(new Uint8Array(32).fill(9), 42), now), e => e.code === 503);
  const fullyAllocated = Array.from({ length: 100 }, (_, i) => ({ ...prior, address: '0x' + (i + 1).toString(16).padStart(64, '0'), nextClaimAt: 0 }));
  assert.throws(() => admission(fullyAllocated, 'TVARA', alias137, now), /pool allocation exhausted/);
  assert.throws(() => normalize('USDC', '0x0000000000000000000000000000000000000000'));
  assert.throws(() => normalize('TVARA', 'not-an-address'));

  const directory = fs.mkdtempSync(path.join(os.tmpdir(), 'isolated-faucet-check-'));
  try {
    const state = path.join(directory, 'journal');
    // Public deterministic TEST key, never funded or used on a network.
    const signer = new Wallet('0x' + '11'.repeat(32));
    const destination = getAddress('0x' + 'ab'.repeat(20));
    const secret = createHash('sha256').update(signer.privateKey).digest();
    const identity = { test: 'offline-signed-original', sender: signer.address };
    let journal = new Journal(state, secret, identity), preparations = 0, attempts = 0;
    const chain = {
      async prepare(asset, address) {
        preparations++;
        assert.equal(asset, 'USDC');
        const raw = await signer.signTransaction({ type: 2, chainId: CHAIN_ID, nonce: 0, to: TOKEN, value: 0, data: ABI.encodeFunctionData('transfer', [address, AMOUNTS.USDC]), gasLimit: 60000, maxFeePerGas: 2000000000n, maxPriorityFeePerGas: 100000000n });
        return { raw, txHash: keccak256(raw), nonce: '0', pin: { height: 7, hash: '0x' + '77'.repeat(32) } };
      },
      validate(claim) {
        const tx = Transaction.from(claim.raw);
        assert(tx.isSigned());
        assert.equal(tx.from, signer.address);
        assert.equal(tx.hash, claim.txHash);
        assert.equal(tx.chainId, BigInt(CHAIN_ID));
        assert.equal(tx.to.toLowerCase(), TOKEN);
        assert.equal(tx.data, ABI.encodeFunctionData('transfer', [claim.address, AMOUNTS.USDC]));
      },
      async broadcast(claim) {
        attempts++;
        // Independently reopen durable authorization BEFORE simulated I/O.
        const saved = new Journal(state, secret, identity).claims;
        assert.equal(saved.length, 1);
        assert.equal(saved[0].raw, claim.raw);
        assert.equal(saved[0].status, 'pending');
        throw Error('Injected transport loss after durable authorization');
      },
    };
    let faucet = new Faucet(journal, chain);
    const results = await Promise.all(Array.from({ length: 12 }, (_, i) => faucet.request('USDC', i % 2 ? destination : destination.toLowerCase())));
    assert.equal(preparations, 1);
    assert.equal(attempts, 1);
    assert(results.every(r => r.status === 'pending' && r.txHash === results[0].txHash && r.nextClaimAt === results[0].nextClaimAt));
    assert.equal(faucet.status('USDC', destination).txHash, results[0].txHash);
    journal = new Journal(state, secret, identity);
    faucet = new Faucet(journal, chain);
    assert.deepEqual(await faucet.request('USDC', destination), results[0]);
    assert.equal(preparations, 1);
    assert.equal(attempts, 1, 'Restart must not broadcast even the original bytes automatically');
    await assert.rejects(faucet.request('USDC', signer.address), e => e.code === 503);
    journal.append({ kind: 'outcome', txHash: results[0].txHash, status: 'failed', inclusion: { height: 9, hash: '0x' + '99'.repeat(32) } });
    assert.match(faucet.status('USDC', destination).error, /daily claim.*consumed/);
    await assert.rejects(faucet.request('USDC', destination.toLowerCase()), e => e.code === 429 && e.nextClaimAt === results[0].nextClaimAt);
    assert.equal(new Journal(state, secret, identity).claims[0].amount, AMOUNTS.USDC);
    const outcome = path.join(state, '000000002.json');
    const outcomeBytes = fs.readFileSync(outcome);
    fs.unlinkSync(outcome);
    assert.throws(() => new Journal(state, secret, identity), /head mismatch|tail rollback/);
    fs.writeFileSync(outcome, outcomeBytes, { mode: 0o600 });

    const original = path.join(state, '000000001.json');
    const before = fs.readFileSync(original, 'utf8');
    const changed = JSON.parse(before);
    changed.body.entry.claim.address = signer.address.toLowerCase();
    fs.writeFileSync(original, JSON.stringify(changed), { mode: 0o600 });
    assert.throws(() => new Journal(state, secret, identity), /authentication failed/);
    await assert.rejects(faucet.request('USDC', destination));
    assert.equal(preparations, 1);
    assert.equal(attempts, 1);
    fs.writeFileSync(original, before, { mode: 0o600 });
    fs.writeFileSync(path.join(state, '000000003.json.new'), 'interrupted original', { mode: 0o600 });
    assert.throws(() => new Journal(state, secret, identity), /Interrupted, missing or unexpected/);
    assert.equal(fs.readFileSync(path.join(state, '000000003.json.new'), 'utf8'), 'interrupted original');
  } finally { fs.rmSync(directory, { recursive: true, force: true }); }
});

test('observation keeps signer ownership and durable progress beyond an aggregate deadline', async (t) => {
  t.mock.timers.enable({ apis: ['setTimeout'] });
  const directory = fs.mkdtempSync(path.join(os.tmpdir(), 'faucet-progress-'));
  try {
    const state = path.join(directory, 'journal');
    const secret = Buffer.alloc(32, 7), identity = { test: 'slow-observation' };
    const original = { asset: 'TVARA', address: '0x' + '11'.repeat(32),
      amount: AMOUNTS.TVARA, status: 'pending', raw: '0x0102', txHash: '0x' + '22'.repeat(32),
      nonce: '0', pin: { height: 100, hash: '0x' + '33'.repeat(32) },
      scanNext: 100, createdAt: 1800000000000, nextClaimAt: 1800000000000 + COOLDOWN };
    const journal = new Journal(state, secret, identity);
    journal.append({ kind: 'original', claim: original });
    const starts = [];
    const chain = {
      validate(claim) { assert.equal(claim.raw, original.raw); },
      async observe(claim) {
        starts.push(claim.scanNext);
        await new Promise(resolve => setTimeout(resolve, 60000));
        return { kind: 'progress', next: claim.scanNext + 32 };
      },
    };
    let faucet = new Faucet(journal, chain);
    const first = faucet.observe('TVARA');
    await Promise.resolve();
    t.mock.timers.tick(45001);
    assert(faucet.jobs.has('TVARA'), 'the in-flight observation still owns the signer');
    t.mock.timers.tick(14999);
    await first;
    const restored = new Journal(state, secret, identity);
    assert.equal(restored.claims[0].scanNext, 132);
    assert.equal(restored.claims[0].txHash, original.txHash);
    assert.equal(restored.claims[0].status, 'pending');
    faucet = new Faucet(restored, chain);
    const second = faucet.observe('TVARA');
    await Promise.resolve();
    t.mock.timers.tick(60000);
    await second;
    assert.deepEqual(starts, [100, 132]);
    assert.equal(new Journal(state, secret, identity).claims[0].scanNext, 164);
  } finally {
    t.mock.timers.reset();
    fs.rmSync(directory, { recursive: true, force: true });
  }
});

// Reconstructed from HTML VB-40 at 7fe1b032dff5143df41b1df22ecaf23c4e2f7cb6.
// Candidate uses the real authorization function; no original PoC bundle claimed.
test('HTML7fe1b032 separate faucet authentication and exact POST origin', () => {
  const { approvedOrigin, authorizeRequest } = require('./test-faucet.cjs');
  const origin = 'https://faucet.example.test';
  assert.equal(approvedOrigin(origin).origin, origin);
  for (const value of ['http://faucet.example.test', 'https://faucet.example.test/path', 'https://u:p@faucet.example.test', 'https://*.example.test'])
    assert.throws(() => approvedOrigin(value));
  const headers = { origin, 'x-faucet-authenticated': 'dedicated-faucet-user' };
  assert.doesNotThrow(() => authorizeRequest({ method: 'POST', headers }, origin));
  assert.doesNotThrow(() => authorizeRequest({ method: 'GET', headers: { 'x-faucet-authenticated': 'dedicated-faucet-user' } }, origin));
  for (const rejected of [
    { ...headers, origin: 'https://funds.example.test' },
    { ...headers, origin: undefined },
    { ...headers, origin: 'null' },
    { origin },
  ]) assert.throws(() => authorizeRequest({ method: 'POST', headers: rejected }, origin), error => error.code === 403);
});
