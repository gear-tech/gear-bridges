import assert from 'node:assert/strict';
import { test } from 'node:test';
import * as evidence from '../lib/eth/finality.js';
import { getEthereumEvidenceConfig } from '../lib/common/env.js';
import { NativeEvidenceClient, EthereumEvidenceError } from '../lib/eth/native-evidence.js';
import { createEvidenceFixture } from './evidence-fixture.mjs';

// Only protocol-fault tests use executables other than the real native authority.
function faultyChild(t, body) {
  const client = new NativeEvidenceClient(process.execPath, ['-e', `
    const readline = require('node:readline');
    readline.createInterface({ input: process.stdin }).on('line', line => {
      const request = JSON.parse(line);
      ${body}
    });
  `]);
  t.after(() => client.close());
  return client;
}

test('native framing faults reject the active request and every queued request', async (t) => {
  for (const [name, body] of [
    ['wrong id', `console.log(JSON.stringify({id:'unknown',ok:true}));`],
    ['numeric id', `console.log(JSON.stringify({id:Number(request.id),ok:true}));`],
    ['empty id', `console.log(JSON.stringify({id:'',ok:true}));`],
    ['missing success fields', `console.log(JSON.stringify({id:request.id,ok:true,op:'qualify'}));`],
    ['wrong operation', `console.log(JSON.stringify({id:request.id,ok:true,op:'receipts',checkpoint:{},receipts:[]}));`],
    ['malformed JSON', `console.log('{');`],
    ['nonobject', `console.log('[]');`],
    ['blank frame', `console.log('');`],
    ['duplicate key', `console.log('{"id":"'+request.id+'","id":"'+request.id+'","ok":true}');`],
    ['invalid UTF-8', `process.stdout.write(Buffer.from([0xff,10]));`],
    ['unknown error kind', `console.log(JSON.stringify({id:request.id,ok:false,error:{kind:'other',message:'bad'}}));`],
    ['extra error field', `console.log(JSON.stringify({id:request.id,ok:false,error:{kind:'conflict',message:'bad'},extra:true}));`],
    ['missing error message', `console.log(JSON.stringify({id:request.id,ok:false,error:{kind:'conflict'}}));`],
    ['oversized frame', `process.stdout.write('x'.repeat(8*1024*1024+1));`],
  ]) {
    await t.test(name, async (t) => {
      const client = faultyChild(t, body);
      const results = await Promise.allSettled([
        client.request({ op: 'qualify', targets: [] }),
        client.request({ op: 'qualify', targets: [] }),
      ]);
      for (const result of results) {
        assert.equal(result.status, 'rejected');
        assert.ok(result.reason instanceof EthereumEvidenceError);
        assert.equal(result.reason.kind, 'protocol');
        assert.equal(result.reason.retryable, false);
      }
      await assert.rejects(client.request({ op: 'qualify', targets: [] }), { kind: 'protocol' });
    });
  }
});

test('native child death and incomplete frames HOLD all queued requests', async (t) => {
  for (const [body, kind] of [
    [`process.exit(7);`, 'unavailable'],
    [`process.stdout.write('{"id":', () => process.exit(0));`, 'protocol'],
  ]) {
    const client = faultyChild(t, body);
    const results = await Promise.allSettled([
      client.request({ op: 'qualify', targets: [] }), client.request({ op: 'qualify', targets: [] }),
    ]);
    for (const result of results) {
      assert.equal(result.status, 'rejected');
      assert.equal(result.reason.kind, kind);
      assert.equal(result.reason.retryable, kind === 'unavailable');
    }
  }
});

test('typed native errors stay fail-closed, redact diagnostics and preserve retry classification', async (t) => {
  for (const kind of ['unavailable', 'conflict', 'protocol']) {
    const client = faultyChild(t, `console.log(JSON.stringify({id:request.id,ok:false,error:{
      kind:${JSON.stringify(kind)},message:'https://secret:password@example.invalid/private'}}));`);
    for (let attempt = 0; attempt < 2; attempt++) {
      await assert.rejects(client.request({ op: 'qualify', targets: [] }), (error) => {
        assert.equal(error.kind, kind);
        assert.equal(error.retryable, kind === 'unavailable');
        assert.match(error.message, /HOLD:/);
        assert.doesNotMatch(error.message, /secret|password|private/);
        return true;
      });
    }
  }
});

test('duplicate replies close the native owner instead of satisfying the next request', async (t) => {
  const client = faultyChild(t, `const frame=JSON.stringify({id:request.id,ok:false,error:{kind:'conflict',message:'rejected'}});
    process.stdout.write(frame+'\\n'+frame+'\\n');`);
  const results = await Promise.allSettled([
    client.request({ op: 'qualify', targets: [] }), client.request({ op: 'qualify', targets: [] }),
  ]);
  assert.equal(results[0].status, 'rejected');
  assert.equal(results[0].reason.kind, 'conflict');
  assert.equal(results[1].status, 'rejected');
  assert.equal(results[1].reason.kind, 'protocol');
});

const hash = (n) => `0x${BigInt(n).toString(16).padStart(64, '0')}`;

test('legacy checkpoint metadata digest matches the shared native canonical JSON vector', () => {
  const checkpoint = {
    schemaVersion: 1, policy: 'independent-two-el-v1', chainId: '31337',
    genesisHash: `0x${'1'.repeat(64)}`, number: '42', timestamp: '1700000000',
    hash: `0x${'2'.repeat(64)}`, parentHash: `0x${'3'.repeat(64)}`,
    stateRoot: `0x${'4'.repeat(64)}`, transactionsRoot: `0x${'5'.repeat(64)}`,
    receiptsRoot: `0x${'6'.repeat(64)}`,
  };
  const saved = structuredClone(checkpoint);
  const expected = '0xb21f8e37958dbd35993aca06421d55228ad6ebb68858172fbc782af43805097d';
  assert.equal(evidence.legacyCheckpointDigest(checkpoint), expected);
  assert.equal(evidence.legacyCheckpointDigest(Object.fromEntries(Object.entries(checkpoint).reverse())), expected);
  assert.notEqual(evidence.legacyCheckpointDigest({ ...checkpoint, stateRoot: `${checkpoint.stateRoot.slice(0, -2)}45` }), expected);
  assert.deepEqual(checkpoint, saved);
});
async function fixture(t, options) {
  const f = await createEvidenceFixture(options);
  t.after(() => f.close());
  f.install();
  Object.defineProperty(f, 'client', { get: () => evidence.createEthereumEvidenceClients('eth') });
  return f;
}
const auth = (f, checkpoint, index = 0) => evidence.authenticateReceipt(f.client, f.txHashes[index],
  { height: 11, hash: f.headers[11].hash }, checkpoint);

test('signed Beacon authority and native MPT proofs authorize the exact original receipt', async (t) => {
  const f = await fixture(t);
  const checkpoint = await evidence.qualifyFinality(f.client);
  assert.equal(checkpoint.number, '11');
  assert.equal(checkpoint.chainId, '31337');
  assert.equal(checkpoint.schemaVersion, 2);
  assert.equal(checkpoint.policy, 'beacon-sync-committee-v1');
  assert.equal(checkpoint.native.schema_version, 2);
  const result = await auth(f, checkpoint);
  assert.equal(result.receipt.transactionHash, f.txHashes[0]);
  assert.equal(result.blockTimestamp, BigInt(f.headers[11].timestamp).toString());
  assert.match(result.inclusionProofDigest, /^0x[0-9a-f]{64}$/);
  assert.match(result.checkpointProofDigest, /^0x[0-9a-f]{64}$/);
  assert.equal(await evidence.authenticateReceipt(f.client, f.txHashes[0], { height: 12 }, checkpoint), undefined);
  assert.ok(f.state.requests.some(({ side }) => side === 'beacon'));
  assert.ok(f.state.requests.some(({ method }) => method === 'eth_getBlockReceipts'));
});

test('EL identity, RLP header corruption and unavailable Beacon evidence HOLD', async (t) => {
  for (const fault of ['chain', 'genesis', 'header', 'timestamp', 'missing', 'beacon', 'signature']) {
    await t.test(fault, async (t) => {
      const f = await fixture(t);
      f.state.fault = ({ side, method, result }) => {
        if (fault === 'chain' && method === 'eth_chainId') return '0x1';
        if (side === 'primary' && result?.number === '0x0' && fault === 'genesis') result.hash = hash(999);
        if (side === 'primary' && result?.number === '0xb') {
          if (fault === 'missing') return null;
          if (fault === 'header') result.stateRoot = hash(999);
          if (fault === 'timestamp') result.timestamp = '0xffffffffffffffff';
        }
        if (side === 'beacon') {
          if (fault === 'beacon') throw new Error('offline');
          if (fault === 'signature' && method.endsWith('/finality_update')) {
            result.data.sync_aggregate.sync_committee_signature = `0x${'00'.repeat(96)}`;
          }
        }
        return result;
      };
      await assert.rejects(evidence.qualifyFinality(f.client), /HOLD:/);
    });
  }
});

test('retained native checkpoints advance without rebinding originals and reject altered projection', async (t) => {
  const f = await fixture(t);
  const previous = await evidence.qualifyFinality(f.client), saved = structuredClone(previous);
  const original = await auth(f, previous);
  f.state.finalizedHeight = 13;
  assert.equal((await evidence.qualifyFinality(f.client, previous)).number, '13');
  assert.deepEqual(previous, saved);
  assert.deepEqual(await auth(f, previous), original);
  for (const field of ['number', 'timestamp', 'hash', 'parentHash', 'stateRoot', 'transactionsRoot', 'receiptsRoot']) {
    const changed = { ...previous, [field]: ['number', 'timestamp'].includes(field) ? '1' : hash(999) };
    await assert.rejects(evidence.qualifyFinality(f.client, changed), /HOLD:/, field);
  }
  const altered = structuredClone(previous);
  altered.native.beacon.proofDigest = hash(999);
  await assert.rejects(evidence.qualifyFinality(f.client, altered), /HOLD:/);
});

test('legacy checkpoints requalify at the original pin and retain separate proof links', async (t) => {
  const f = await fixture(t);
  const header = f.headers[10];
  const legacy = { schemaVersion: 1, policy: 'independent-two-el-v1', chainId: '31337',
    genesisHash: f.headers[0].hash, number: '10', timestamp: BigInt(header.timestamp).toString(),
    ...Object.fromEntries(['hash', 'parentHash', 'stateRoot', 'transactionsRoot', 'receiptsRoot'].map((key) => [key, header[key]])) };
  const saved = structuredClone(legacy);
  const checkpoint = await evidence.qualifyFinality(f.client, legacy);
  assert.equal(checkpoint.schemaVersion, 2);
  assert.deepEqual(legacy, saved);
  const original = await auth(f, checkpoint);
  assert.ok(original.requalificationLinks.length > 0);
  for (const link of original.requalificationLinks) {
    assert.match(link.originalDigest, /^0x[0-9a-f]{64}$/);
    assert.match(link.proofDigest, /^0x[0-9a-f]{64}$/);
  }
  const retainedCheckpoint = structuredClone(checkpoint);
  f.state.finalizedHeight = 13;
  const advanced = await evidence.qualifyFinality(f.client, checkpoint, [], [legacy]);
  const retainedLinks = original.requalificationLinks.map((link) => ({ ...link, checkpoint: checkpoint.native }));
  assert.equal((await evidence.qualifyFinality(f.client, advanced, [], [legacy], retainedLinks)).number, '13');
  assert.deepEqual(checkpoint, retainedCheckpoint);
  assert.deepEqual(retainedLinks, original.requalificationLinks.map((link) => ({ ...link, checkpoint: retainedCheckpoint.native })));
  await assert.rejects(evidence.qualifyFinality(f.client, { ...legacy, receiptsRoot: hash(999) }), /HOLD:/);
});

test('lifetime qualification crosses the native frame cap without dropping fences or rebinding links', async (t) => {
  const f = await fixture(t);
  const prior = await evidence.qualifyFinality(f.client);
  const header = f.headers[10];
  const legacy = { schemaVersion: 1, policy: 'independent-two-el-v1', chainId: '31337',
    genesisHash: f.headers[0].hash, number: '10', timestamp: BigInt(header.timestamp).toString(),
    hash: header.hash, parentHash: header.parentHash, stateRoot: header.stateRoot,
    transactionsRoot: header.transactionsRoot, receiptsRoot: header.receiptsRoot };
  const oldQualification = await evidence.qualifyFinality(f.client, legacy);
  const oldOriginal = await auth(f, oldQualification);
  const retainedLinks = oldOriginal.requalificationLinks.map((link) => ({ ...link, checkpoint: oldQualification.native }));
  // Repeat valid native records to cross the wire limit without thousands of
  // unrelated fixture signatures. Each supplied record must still be checked.
  const copies = Math.ceil(9 * 1024 * 1024 / Buffer.byteLength(JSON.stringify(prior.native)));
  const retained = [...Array.from({ length: copies }, () => prior), legacy];
  assert.ok(Buffer.byteLength(JSON.stringify({ checkpointFences: retained.map((item) => item.native ?? item) })) > 8 * 1024 * 1024);
  const request = f.client.native.request.bind(f.client.native), frames = [];
  let pinned;
  t.mock.method(f.client.native, 'request', async (input) => {
    frames.push(structuredClone(input));
    assert.ok(Buffer.byteLength(JSON.stringify(input)) < 2 * 1024 * 1024);
    const response = await request(input);
    assert.ok(Buffer.byteLength(JSON.stringify(response)) < 2 * 1024 * 1024);
    if (input.op === 'qualify') {
      pinned ??= structuredClone(response.checkpoint);
      assert.deepEqual(response.checkpoint, pinned, 'all chunks retain the initial view even when Beacon advances');
      f.state.finalizedHeight = 13;
    }
    return response;
  });
  const saved = structuredClone(retainedLinks);
  const checkpoint = await evidence.qualifyFinality(f.client, undefined, [], retained, retainedLinks);
  assert.equal(checkpoint.number, '11');
  const batches = frames.filter((frame) => frame.op === 'qualify' && frame.fixedCheckpoint);
  assert.ok(batches.length > 1);
  assert.equal(batches.flatMap((frame) => frame.checkpointFences ?? []).filter((item) => item.schema_version === 2).length, copies);
  assert.deepEqual(retainedLinks, saved);
  const original = await auth(f, checkpoint);
  assert.deepEqual(original.requalificationLinks, oldOriginal.requalificationLinks);
  assert.equal(original.checkpointProofDigest, prior.native.beacon.proofDigest);
  // A conflict in the last frame must not be hidden by successful earlier chunks.
  const corrupt = structuredClone(prior);
  corrupt.native.beacon.proofDigest = hash(999);
  pinned = undefined;
  await assert.rejects(evidence.qualifyFinality(f.client, undefined, [], [...retained, corrupt], retainedLinks), /HOLD/);
});

test('retained continuity verifies an intervening header, not only endpoint identity', async (t) => {
  const f = await fixture(t), previous = await evidence.qualifyFinality(f.client);
  f.state.finalizedHeight = 13;
  f.state.fault = ({ result }) => result?.number === '0xc' ? { ...result, parentHash: hash(999) } : result;
  await assert.rejects(evidence.qualifyFinality(f.client, previous), /HOLD:/);
});

test('long native history walks have no aggregate JavaScript five-minute ceiling', async (t) => {
  const f = await fixture(t, { finalizedHeight: 11, finalizedHeights: [11, 3613] });
  const previous = await evidence.qualifyFinality(f.client), saved = structuredClone(previous);
  let now = Date.now(), elapsed = 0;
  t.mock.method(Date, 'now', () => now);
  f.state.fault = ({ result }) => { now += 100; elapsed += 100; return result; };
  f.state.finalizedHeight = 3613;
  assert.equal((await evidence.qualifyFinality(f.client, previous)).number, '3613');
  assert.ok(elapsed > 300_000);
  assert.deepEqual(previous, saved);
});

test('stalled native upstream requests retain their individual timeout', { timeout: 45_000 }, async (t) => {
  const f = await fixture(t);
  f.state.fault = ({ side, result }) => side === 'beacon' ? new Promise(() => {}) : result;
  await assert.rejects(evidence.qualifyFinality(f.client), (error) => error.kind === 'unavailable' && error.retryable);
});

test('receipt trie corruption and missing complete receipt sets never authorize effects', async (t) => {
  const mutations = [
    (r) => { r.status = '0x0'; }, (r) => { r.type = '0x1'; },
    (r) => { r.cumulativeGasUsed = '0x5209'; }, (r) => { r.logsBloom = `0x${'11'.repeat(256)}`; },
    (r) => { r.transactionHash = hash(999); }, (r) => { r.transactionIndex = '0x1'; },
    (r) => { r.blockHash = hash(999); }, (r) => { r.blockNumber = '0x1'; },
    (r) => { r.logs[0].data = '0x'; }, (r) => { r.logs[0].address = `0x${'99'.repeat(20)}`; },
    (r) => { r.logs[0].topics = [hash(999)]; }, (r) => { r.logs.push(structuredClone(r.logs[0])); },
    (r) => { r.logs[0].address = '0x11'; }, (r) => { r.logs[0].topics = ['0x11']; },
    (r) => { r.logs[0].data = '0xabc'; }, (r) => { r.status = '0x2'; }, (r) => { delete r.logsBloom; },
    (r) => { r.logs[0].removed = true; }, (r) => { r.logs[0].logIndex = '0xffffffffffffffff'; },
    (r) => { r.logs[0].transactionHash = hash(999); }, (r) => { r.logs[0].blockHash = hash(999); },
    (r) => { r.logs[0].blockNumber = '0x1'; }, (r) => { r.logs[0].transactionIndex = '0x1'; },
  ];
  for (const [index, mutate] of mutations.entries()) {
    await t.test(String(index), async (t) => {
      const f = await fixture(t), checkpoint = await evidence.qualifyFinality(f.client);
      f.state.fault = ({ method, result }) => {
        if (method === 'eth_getBlockReceipts') mutate(result[0]);
        return result;
      };
      await assert.rejects(auth(f, checkpoint), /HOLD:/);
    });
  }
  for (const replacement of [null, []]) {
    await t.test(JSON.stringify(replacement), async (t) => {
      const f = await fixture(t), checkpoint = await evidence.qualifyFinality(f.client);
      f.state.fault = ({ method, result }) => method === 'eth_getBlockReceipts' ? replacement : result;
      await assert.rejects(auth(f, checkpoint), /HOLD:/);
    });
  }
});

test('native inclusion derives global log coordinates from proven receipt prefixes', async (t) => {
  const f = await fixture(t), checkpoint = await evidence.qualifyFinality(f.client);
  f.state.fault = ({ method, result }) => {
    if (method === 'eth_getBlockReceipts') for (const receipt of result) {
      receipt.providerDecoration = 'not evidence';
      receipt.transactionHash = receipt.transactionHash.toUpperCase().replace('0X', '0x');
      for (const log of receipt.logs) log.data = log.data.toUpperCase().replace('0X', '0x');
    }
    return result;
  };
  const result = await auth(f, checkpoint, 1);
  assert.equal(result.receipt.logs[0].logIndex, '0x3');
  assert.equal(result.receipt.logs[0].transactionIndex, '0x1');
  assert.equal(result.receipt.logs[0].transactionHash, f.txHashes[1]);
  assert.equal(result.receipt.logs[0].blockHash, f.headers[11].hash);
});

test('missing originals HOLD; immutable retained proofs remain usable but corruption does not', async (t) => {
  const f = await fixture(t), checkpoint = await evidence.qualifyFinality(f.client);
  await assert.rejects(evidence.authenticateReceipt(f.client, hash(999), { height: 11 }, checkpoint), /HOLD:/);
  const original = await auth(f, checkpoint);
  f.state.fault = ({ method, result }) => method === 'eth_getBlockReceipts' ? null : result;
  assert.deepEqual(await auth(f, checkpoint), original);
  await f.corruptProof(f.txHashes[0]);
  await assert.rejects(auth(f, checkpoint), /HOLD:/);
});

test('receipt authenticator remains callable, coalesces within an attempt and keys exact inclusion', async (t) => {
  const f = await fixture(t), checkpoint = await evidence.qualifyFinality(f.client);
  const cached = evidence.createReceiptAuthenticator(f.client, checkpoint);
  const [a, b] = await Promise.all([cached(f.txHashes[0], { height: 11 }), cached(f.txHashes[0], { height: 11 })]);
  assert.equal(a, b);
  assert.equal(f.state.requests.filter(({ method }) => method === 'eth_getBlockReceipts').length, 1);
  await assert.rejects(cached(f.txHashes[0], { height: 11, hash: hash(999) }), /HOLD:/);
  const expected = { height: 11, hash: f.headers[11].hash, proofDigest: a.inclusionProofDigest };
  const pinned = await cached(f.txHashes[0], expected);
  assert.deepEqual(pinned, a);
  assert.equal(await cached(f.txHashes[0], { ...expected }), pinned);
  await assert.rejects(cached(f.txHashes[0], { ...expected, proofDigest: hash(999) }), /HOLD:/);
  await f.corruptProof(f.txHashes[0]);
  assert.equal(await cached(f.txHashes[0], { height: 11 }), a);
  assert.equal(await cached(f.txHashes[0], expected), pinned);
  await assert.rejects(evidence.createReceiptAuthenticator(f.client, checkpoint)(f.txHashes[0], { height: 11 }), /HOLD:/);
  await assert.rejects(evidence.createReceiptAuthenticator(f.client, checkpoint)(f.txHashes[0], expected), /HOLD:/);
});

test('one persistent native owner verifies every grouped page fence, including middle targets', async (t) => {
  const f = await fixture(t);
  const client = f.client;
  const targets = [0, 5, 10, 11].map((number) => ({ number: String(number), hash: f.headers[number].hash }));
  const checkpoint = await evidence.qualifyFinality(client, undefined, targets);
  assert.equal(f.client, client);
  assert.equal((await evidence.qualifyFinality(client, checkpoint, targets)).hash, checkpoint.hash);
  await assert.rejects(evidence.qualifyFinality(client, checkpoint,
    targets.map((target) => target.number === '5' ? { ...target, hash: hash(999) } : target)), /HOLD:/);
});

test('complete transaction data must match the signed execution transaction root', async (t) => {
  for (const fault of ['missing', 'changed']) {
    await t.test(fault, async (t) => {
      const f = await fixture(t), checkpoint = await evidence.qualifyFinality(f.client);
      f.state.fault = ({ method, params, result }) => {
        if (method === 'eth_getBlockByHash' && params[1] && result?.number === '0xb') {
          if (fault === 'missing') result.transactions = [];
          else result.transactions[0].input = '0x1234';
        }
        return result;
      };
      await assert.rejects(auth(f, checkpoint), /HOLD:/);
    });
  }
});

test('bootstrap approval digest is independently checked by the actual child', async (t) => {
  const f = await fixture(t);
  f.env.ETH_FINALITY_BOOTSTRAP_SHA256 = hash(999);
  f.install();
  await assert.rejects(evidence.qualifyFinality(f.client), /HOLD:/);
});

function store(rows = [], insertionError) {
  return { inserted: [], async find() { return rows; }, async findOneBy(_, { id }) { return rows.find((row) => row.id === id); },
    async insert(row) { if (insertionError) throw insertionError; this.inserted.push(row); } };
}
function row(checkpoint) {
  return { id: evidence.checkpointId(checkpoint), blockNumber: BigInt(checkpoint.number), checkpoint };
}
test('checkpoint loading checks every row identity before choosing highest', async (t) => {
  const f = await fixture(t), lower = await evidence.qualifyFinality(f.client);
  f.state.finalizedHeight = 13;
  const checkpoint = await evidence.qualifyFinality(f.client, lower);
  const other = { ...checkpoint, chainId: '2' };
  await assert.rejects(evidence.loadCheckpoint(store([row(checkpoint), { ...row(checkpoint), checkpoint: other }]), f.client));
  assert.deepEqual(await evidence.loadCheckpoint(store([row(lower), row(checkpoint)]), f.client), checkpoint);
  assert.deepEqual(await evidence.loadCheckpoint(store([{ ...row(checkpoint), blockNumber: checkpoint.number }]), f.client), checkpoint);
  await assert.rejects(evidence.loadCheckpoint(store([{ ...row(checkpoint), id: 'wrong' }]), f.client));
  await assert.rejects(evidence.loadCheckpoint(store([{ ...row(checkpoint), blockNumber: 2n }]), f.client));
});

test('checkpoint insertion compares complete immutable record and retries only its own unique race', async (t) => {
  const f = await fixture(t), checkpoint = await evidence.qualifyFinality(f.client);
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
  await assert.rejects(evidence.retainCheckpoint(store([{ ...row(checkpoint), checkpoint: changed }]), checkpoint));
  const race = Object.assign(new Error('duplicate'), { code: '23505', constraint: 'ethereum_finality_checkpoint_pkey' });
  await assert.rejects(evidence.retainCheckpoint(store([], race), checkpoint), (error) => error.code === '40001' && error.cause === race);
  const unrelated = Object.assign(new Error('duplicate'), { code: '23505', constraint: 'another_pkey' });
  await assert.rejects(evidence.retainCheckpoint(store([], unrelated), checkpoint), (error) => error === unrelated);
});

test('native environment requires approved identity and an exclusive archive per owner', () => {
  const valid = {
    ETH_RPC_URL: 'http://127.0.0.1:8545', ETH_FINALITY_BEACON_RPC: 'http://127.0.0.1:5052',
    ETH_CHAIN_ID: '31337', ETH_GENESIS_HASH: hash(1), ETH_EVIDENCE_BINARY: '/tmp/relayer',
    ETH_FINALITY_BOOTSTRAP: '/tmp/bootstrap.json', ETH_FINALITY_BOOTSTRAP_SHA256: hash(2),
    ETH_FINALITY_ARCHIVE_ROOT: '/tmp/evidence-archive',
  };
  const before = Object.fromEntries(Object.keys(valid).map((key) => [key, process.env[key]]));
  try {
    Object.assign(process.env, valid);
    for (const owner of ['eth', 'gear', 'repair']) {
      const config = getEthereumEvidenceConfig(owner);
      assert.equal(config.chainId, 31337n);
      assert.equal(config.store, `${valid.ETH_FINALITY_ARCHIVE_ROOT}/${owner}`);
      assert.equal(config.binary, valid.ETH_EVIDENCE_BINARY);
      assert.deepEqual(config.proofSources, ['eth', 'gear', 'repair'].filter((peer) => peer !== owner)
        .map((peer) => `${valid.ETH_FINALITY_ARCHIVE_ROOT}/${peer}/beacon`));
    }
    for (const [key, value] of [
      ['ETH_CHAIN_ID', '01'], ['ETH_CHAIN_ID', '-1'], ['ETH_CHAIN_ID', '18446744073709551616'],
      ['ETH_GENESIS_HASH', '0x1234'], ['ETH_FINALITY_BOOTSTRAP_SHA256', hash(2).toUpperCase()],
      ['ETH_FINALITY_BOOTSTRAP', ''], ['ETH_FINALITY_ARCHIVE_ROOT', ''],
    ]) {
      Object.assign(process.env, valid, { [key]: value });
      assert.throws(() => getEthereumEvidenceConfig(), key);
    }
    for (const key of ['ETH_RPC_URL', 'ETH_FINALITY_BEACON_RPC']) {
      for (const endpoint of ['ws://127.0.0.1:8545', 'http://rpc.example',
        'http://192.168.1.2:8545', 'http://[2001:db8::1]:8545', 'http://localhost.example:8545',
        'http://127.0.0.1.example:8545', 'https://rpc.example/#fragment']) {
        Object.assign(process.env, valid, { [key]: endpoint });
        assert.throws(() => getEthereumEvidenceConfig(), `${key}=${endpoint}`);
      }
      for (const endpoint of ['https://rpc.example:443/rpc', 'http://127.0.0.1:8545', 'http://[::1]:8545']) {
        Object.assign(process.env, valid, { [key]: endpoint });
        assert.equal(getEthereumEvidenceConfig().chainId, 31337n);
      }
    }
  } finally {
    for (const [key, value] of Object.entries(before)) {
      if (value === undefined) delete process.env[key]; else process.env[key] = value;
    }
  }
});
