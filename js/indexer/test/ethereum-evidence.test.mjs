import assert from 'node:assert/strict';
import { test } from 'node:test';
import * as evidence from '../lib/eth/finality.js';
import { getEthereumEvidenceConfig } from '../lib/common/env.js';

const hash = (n) => `0x${BigInt(n).toString(16).padStart(64, '0')}`;
const address = `0x${'11'.repeat(20)}`;
const txHash = hash(500);
const hex = (n) => `0x${n.toString(16)}`;
function fixture(primaryTip = 4, witnessTip = 3) {
  const chain = Array.from({ length: 6 }, (_, n) => ({ number: hex(n), hash: hash(n + 1),
    parentHash: hash(n), stateRoot: hash(100 + n), transactionsRoot: hash(200 + n),
    receiptsRoot: hash(300 + n), timestamp: hex(1_700_000_000 + n), transactions: [txHash] }));
  const receipt = { transactionHash: txHash, blockHash: chain[2].hash, blockNumber: '0x2',
    transactionIndex: '0x0', type: '0x2', cumulativeGasUsed: '0x5208', logsBloom: `0x${'00'.repeat(256)}`,
    status: '0x1', logs: [{ address, topics: [hash(900)], data: '0xabcd', logIndex: '0x7',
      blockNumber: '0x2', blockHash: chain[2].hash, transactionHash: txHash, transactionIndex: '0x0', removed: false }] };
  const calls = [];
  const primary = { chain: structuredClone(chain), receipt: structuredClone(receipt), tip: primaryTip, chainId: '0x1' };
  const witness = { chain: structuredClone(chain), receipt: structuredClone(receipt), tip: witnessTip, chainId: '0x1' };
  const call = (side) => async (method, params) => {
    calls.push({ side, method, params });
    if (side.error) throw side.error;
    if (method === 'eth_chainId') return side.chainId;
    if (method === 'eth_getTransactionReceipt') return structuredClone(side.receipt);
    if (method === 'eth_getBlockByNumber') {
      const n = params[0] === 'finalized' ? side.tip : Number(BigInt(params[0]));
      return structuredClone(side.chain[n] ?? null);
    }
    throw new Error(`Unexpected RPC ${method}`);
  };
  const clients = { primaryCall: call(primary), witnessCall: call(witness), chainId: 1n, genesisHash: chain[0].hash };
  return { clients, primary, witness, chain, receipt, calls };
}
const auth = (f, checkpoint) => evidence.authenticateReceipt(f.clients, txHash, { height: 2, hash: f.chain[2].hash }, checkpoint);

test('lower finalized tip is shared authority; receipts above it stay pending', async () => {
  const f = fixture();
  const checkpoint = await evidence.qualifyFinality(f.clients);
  assert.equal(checkpoint.number, '3');
  assert.equal(checkpoint.chainId, '1');
  assert.equal(checkpoint.policy, 'independent-two-el-v1');
  assert.equal((await auth(f, checkpoint)).blockTimestamp, '1700000002');
  assert.equal(await evidence.authenticateReceipt(f.clients, txHash, { height: 4 }, checkpoint), undefined);
});

test('network identity, same-height forks, missing headers and ancestry contradictions HOLD', async () => {
  for (const mutate of [
    (f) => { f.witness.chainId = '0x2'; },
    (f) => { f.witness.chain[0].hash = hash(999); },
    (f) => { f.witness.chain[3].hash = hash(999); },
    (f) => { f.witness.chain[3].stateRoot = hash(999); },
    (f) => { f.primary.chain[4].parentHash = hash(999); },
    (f) => { f.primary.chain[3] = null; },
    (f) => { f.witness.error = new Error('offline'); },
    (f) => { f.primary.chain[3].timestamp = '0xffffffffffffffff'; },
  ]) {
    const f = fixture(); mutate(f);
    await assert.rejects(evidence.qualifyFinality(f.clients));
  }
});

test('retained checkpoint cannot regress, change fields, or fall off ancestry', async () => {
  const f = fixture();
  const previous = await evidence.qualifyFinality(f.clients);
  f.witness.tip = 2;
  await assert.rejects(evidence.qualifyFinality(f.clients, previous));
  f.witness.tip = 4;
  assert.equal((await evidence.qualifyFinality(f.clients, previous)).number, '4');
  f.primary.chain[3].receiptsRoot = f.witness.chain[3].receiptsRoot = hash(999);
  await assert.rejects(evidence.qualifyFinality(f.clients, previous));
});

test('retained continuity walks every intervening parent, not only its endpoint', async () => {
  const f = fixture(3, 3);
  const previous = await evidence.qualifyFinality(f.clients);
  f.primary.tip = f.witness.tip = 5;
  f.primary.chain[4].parentHash = f.witness.chain[4].parentHash = hash(999);
  await assert.rejects(evidence.qualifyFinality(f.clients, previous));
});

test('fresh finality pin must survive post-acquisition explicit-height recheck', async () => {
  const f = fixture();
  const original = f.clients.witnessCall;
  let reads = 0;
  f.clients.witnessCall = async (method, params) => {
    const answer = await original(method, params);
    if (method === 'eth_getBlockByNumber' && params[0] === '0x3' && ++reads >= 2) answer.hash = hash(999);
    return answer;
  };
  await assert.rejects(evidence.qualifyFinality(f.clients));
});

test('matching headers do not authenticate one-provider receipt status, effects or coordinates', async () => {
  for (const mutate of [
    (r) => { r.status = '0x0'; }, (r) => { r.type = '0x1'; },
    (r) => { r.cumulativeGasUsed = '0x5209'; }, (r) => { r.logsBloom = `0x${'11'.repeat(256)}`; },
    (r) => { r.transactionHash = hash(999); }, (r) => { r.transactionIndex = '0x1'; },
    (r) => { r.blockHash = hash(999); }, (r) => { r.blockNumber = '0x1'; },
    (r) => { r.logs[0].data = '0x'; }, (r) => { r.logs[0].address = `0x${'22'.repeat(20)}`; },
    (r) => { r.logs[0].topics = [hash(999)]; }, (r) => { r.logs[0].logIndex = '0x8'; },
    (r) => { r.logs[0].transactionHash = hash(999); }, (r) => { r.logs[0].blockHash = hash(999); },
    (r) => { r.logs[0].blockNumber = '0x1'; }, (r) => { r.logs[0].transactionIndex = '0x1'; },
  ]) {
    const f = fixture(); const checkpoint = await evidence.qualifyFinality(f.clients);
    mutate(f.witness.receipt);
    await assert.rejects(auth(f, checkpoint));
  }
});

test('both providers must return valid ordered receipt coordinates and exact transaction membership', async () => {
  for (const mutate of [
    (r) => { r.logs[0].removed = true; },
    (r) => { r.logs.push(structuredClone(r.logs[0])); },
    (r) => { r.logs.push({ ...r.logs[0], logIndex: '0x9' }); },
    (r) => { r.logs[0].logIndex = '0xffffffffffffffff'; },
    (r) => { r.logs[0].address = '0x11'; },
    (r) => { r.logs[0].topics = ['0x11']; },
    (r) => { r.logs[0].data = '0xabc'; },
    (r) => { r.status = '0x2'; },
    (r) => { delete r.logsBloom; },
  ]) {
    const f = fixture(); const checkpoint = await evidence.qualifyFinality(f.clients);
    mutate(f.primary.receipt); mutate(f.witness.receipt);
    await assert.rejects(auth(f, checkpoint));
  }
  for (const side of ['primary', 'witness']) {
    const f = fixture(); const checkpoint = await evidence.qualifyFinality(f.clients);
    f[side].chain[2].transactions = [hash(999)];
    await assert.rejects(auth(f, checkpoint));
  }
});

test('receipt presence must agree and historical authentication rechecks the fixed pin', async () => {
  const f = fixture(); const checkpoint = await evidence.qualifyFinality(f.clients);
  f.primary.receipt = null;
  await assert.rejects(auth(f, checkpoint));
  f.witness.receipt = null;
  assert.equal(await auth(f, checkpoint), undefined);
  f.primary.receipt = f.witness.receipt = structuredClone(f.receipt);
  f.primary.chain[3].parentHash = f.witness.chain[3].parentHash = hash(999);
  await assert.rejects(auth(f, checkpoint));
});

test('historical receipt authentication uses bounded explicit headers, not genesis-to-tip replay', async () => {
  const f = fixture(5, 5);
  const checkpoint = await evidence.qualifyFinality(f.clients);
  f.calls.length = 0;
  const original = await auth(f, checkpoint);
  assert.equal(original.receipt.transactionHash, txHash);
  const blocks = f.calls.filter((call) => call.method === 'eth_getBlockByNumber');
  assert.equal(blocks.length, 6);
  assert.ok(blocks.every(({ params }) => params[0] === '0x2' || params[0] === '0x5'));
});

test('normalization tolerates casing, quantities and unused decorations without dropping authenticated fields', async () => {
  const f = fixture(); const checkpoint = await evidence.qualifyFinality(f.clients);
  f.witness.receipt.status = '0x01';
  f.witness.receipt.transactionHash = txHash.toUpperCase().replace('0X', '0x');
  f.witness.receipt.logs[0].data = '0xABCD';
  f.witness.receipt.providerDecoration = 'ignored';
  const result = await auth(f, checkpoint);
  assert.deepEqual(result.receipt, f.receipt);
});

test('receipt cache belongs to one attempt and keys expected inclusion identity', async () => {
  const f = fixture(); const checkpoint = await evidence.qualifyFinality(f.clients);
  const cached = evidence.createReceiptAuthenticator(f.clients, checkpoint);
  const [a, b] = await Promise.all([cached(txHash, { height: 2 }), cached(txHash, { height: 2 })]);
  assert.equal(a, b);
  assert.equal(f.calls.filter((c) => c.method === 'eth_getTransactionReceipt').length, 2);
  await assert.rejects(cached(txHash, { height: 2, hash: hash(999) }));
  await evidence.createReceiptAuthenticator(f.clients, checkpoint)(txHash, { height: 2 });
  assert.equal(f.calls.filter((c) => c.method === 'eth_getTransactionReceipt').length, 6);
});

function store(rows = [], insertionError) {
  return { inserted: [], async find() { return rows; }, async findOneBy(_, { id }) { return rows.find((row) => row.id === id); },
    async insert(row) { if (insertionError) throw insertionError; this.inserted.push(row); } };
}
function row(checkpoint) {
  return { id: evidence.checkpointId(checkpoint), blockNumber: BigInt(checkpoint.number), checkpoint };
}
test('checkpoint loading checks every row identity before choosing highest', async () => {
  const f = fixture(); const checkpoint = await evidence.qualifyFinality(f.clients);
  const other = { ...checkpoint, chainId: '2', number: '1' };
  await assert.rejects(evidence.loadCheckpoint(store([row(checkpoint), row(other)]), f.clients));
  const lower = { ...checkpoint, number: '1' };
  assert.deepEqual(await evidence.loadCheckpoint(store([row(lower), row(checkpoint)]), f.clients), checkpoint);
  assert.deepEqual(await evidence.loadCheckpoint(store([{ ...row(checkpoint), blockNumber: checkpoint.number }]), f.clients), checkpoint);
  await assert.rejects(evidence.loadCheckpoint(store([{ ...row(checkpoint), id: 'wrong' }]), f.clients));
  await assert.rejects(evidence.loadCheckpoint(store([{ ...row(checkpoint), blockNumber: 2n }]), f.clients));
});

test('checkpoint insertion compares complete record without upsert and retries only its own unique race', async () => {
  const f = fixture(); const checkpoint = await evidence.qualifyFinality(f.clients);
  const same = store([row(checkpoint)]);
  await evidence.retainCheckpoint(same, checkpoint);
  assert.equal(same.inserted.length, 0);
  const postgres = store([{ ...row(checkpoint), blockNumber: checkpoint.number,
    checkpoint: Object.fromEntries(Object.entries(checkpoint).reverse()) }]);
  await evidence.retainCheckpoint(postgres, checkpoint);
  assert.equal(postgres.inserted.length, 0);
  const empty = store(); await evidence.retainCheckpoint(empty, checkpoint);
  assert.equal(empty.inserted.length, 1);
  const changed = { ...checkpoint, stateRoot: hash(999) };
  await assert.rejects(evidence.retainCheckpoint(store([row(changed)]), checkpoint));
  const race = Object.assign(new Error('duplicate'), { code: '23505', constraint: 'ethereum_finality_checkpoint_pkey' });
  await assert.rejects(evidence.retainCheckpoint(store([], race), checkpoint), (error) => error.code === '40001' && error.cause === race);
  const unrelated = Object.assign(new Error('duplicate'), { code: '23505', constraint: 'another_pkey' });
  await assert.rejects(evidence.retainCheckpoint(store([], unrelated), checkpoint), (error) => error === unrelated);
});

test('RPC and shared ancestry deadlines HOLD rather than bypassing witness', async (t) => {
  const f = fixture();
  t.mock.method(globalThis, 'setTimeout', (fn) => { queueMicrotask(fn); return 0; });
  f.clients.witnessCall = () => new Promise(() => {});
  await assert.rejects(evidence.qualifyFinality(f.clients), /HOLD/);
  t.mock.restoreAll();
  const g = fixture(); let now = 0;
  t.mock.method(Date, 'now', () => (now += 300_001));
  await assert.rejects(evidence.qualifyFinality(g.clients), /HOLD/);
});

test('shared environment requires two distinct HTTP endpoints and approved exact identity', () => {
  const keys = ['ETH_RPC_URL', 'ETH_WITNESS_RPC', 'ETH_CHAIN_ID', 'ETH_GENESIS_HASH'];
  const before = keys.map((key) => process.env[key]);
  try {
    const valid = { ETH_RPC_URL: 'http://localhost:8545', ETH_WITNESS_RPC: 'http://localhost:8546', ETH_CHAIN_ID: '1', ETH_GENESIS_HASH: hash(1) };
    Object.assign(process.env, valid);
    assert.equal(getEthereumEvidenceConfig().chainId, 1n);
    for (const [key, value] of [ ['ETH_WITNESS_RPC', ''], ['ETH_WITNESS_RPC', 'http://LOCALHOST:8545/'],
      ['ETH_RPC_URL', 'ws://localhost:8545'], ['ETH_CHAIN_ID', '01'], ['ETH_CHAIN_ID', '-1'],
      ['ETH_GENESIS_HASH', '0x1234'] ]) {
      Object.assign(process.env, valid, { [key]: value });
      assert.throws(() => getEthereumEvidenceConfig());
    }
  } finally {
    keys.forEach((key, i) => { if (before[i] === undefined) delete process.env[key]; else process.env[key] = before[i]; });
  }
});
