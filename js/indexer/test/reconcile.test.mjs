import assert from 'node:assert/strict';
import { test } from 'node:test';
import { fileURLToPath } from 'node:url';
import { createEvidenceFixture } from './evidence-fixture.mjs';
// Exercise the CLI import boundary with only its ABI location, not processor configuration.
process.env.ETH_API_PATH = fileURLToPath(new URL('../../../api/ethereum', import.meta.url));
const { parseReconcileArgs, reconcilePage } = await import('../lib/eth/reconcile.js');
const { Transfer, CompletedTransfer, EthereumFinalityCheckpoint, ReceiptRelay, ReceiptSettlement, Network, Status } = await import('../lib/model/index.js');
const manager = await import('../lib/eth/abi/erc20-manager.js');
const queue = await import('../lib/eth/abi/message-queue.js');

const hash = (byte) => `0x${byte.repeat(32)}`;
const addr = (byte) => `0x${byte.repeat(20)}`;
const queueAddress = addr('22'), managerAddress = addr('11');
const options = { direction: 'vara-to-eth', limit: 1, apply: false };
function storeOf(...initial) {
  const rows = new Map(), writes = [], queries = [];
  const copy = (row) => Object.assign(new row.constructor(), structuredClone(row));
  for (const row of initial) {
    if (!rows.has(row.constructor)) rows.set(row.constructor, new Map());
    rows.get(row.constructor).set(row.id, copy(row));
  }
  const matches = (row, where) => !where || (Array.isArray(where) ? where.some((w) => matches(row, w)) :
    Object.entries(where).every(([key, expected]) => {
      const equal = (v) => {
        if (v?.type === 'in') return v.value.some((x) => String(x) === String(row[key]));
        if (v?.type === 'moreThan') return row[key] > v.value;
        if (v?.type === 'not') return !equal(v.value);
        return String(row[key]) === String(v);
      };
      return equal(expected);
    }));
  return { rows, writes, queries,
    snapshot() { return [...rows].map(([entity, values]) => [entity.name, structuredClone([...values])]); },
    async find(entity, opts) {
      queries.push({ entity, opts });
      let result = [...(rows.get(entity)?.values() ?? [])].filter((r) => matches(r, opts?.where));
      for (const [key, order] of Object.entries(opts?.order ?? {}).reverse()) {
        result.sort((a, b) => (a[key] < b[key] ? -1 : a[key] > b[key] ? 1 : 0) * (order === 'DESC' ? -1 : 1));
      }
      return result.slice(0, opts?.take ?? result.length).map(copy);
    },
    async findOneBy(entity, where) { return (await this.find(entity, { where }))[0]; },
    async save(items) {
      for (const row of Array.isArray(items) ? items : [items]) {
        writes.push(row);
        if (!rows.has(row.constructor)) rows.set(row.constructor, new Map());
        rows.get(row.constructor).set(row.id, copy(row));
      }
    },
    async insert(items) { await this.save(items); },
  };
}
async function fixture(t, specifications, finalizedHeight = 2) {
  const event = (abi, name, args, address) => ({ ...abi.encodeEventLog(abi.getEvent(name), args), address });
  const native = await createEvidenceFixture({ finalizedHeight, finalizedHeights: [0, 1, 2, 3],
    receipts: specifications ?? [{ blockNumber: 1, transactionIndex: 0, logs: [
      event(manager.abi, 'Bridged', [hash('44'), addr('55'), addr('66'), 10n], managerAddress),
      event(queue.abi, 'MessageProcessed', [9n, hash('77'), 8n, managerAddress], queueAddress),
    ] }] });
  t.after(() => native.close());
  const receipt = native.receipts.at(-1);
  const transfer = new Transfer({ id: hash('10'), nonce: '8', txHash: hash('ee'), blockNumber: 9n,
    sourceNetwork: Network.Vara, destNetwork: Network.Ethereum, status: Status.Bridging,
    bridgingStartedAtBlock: 9n, ethBridgeBuiltInMsgHash: hash('77'), sender: hash('44'), receiver: addr('55'),
    source: hash('66'), destination: addr('66'), amount: '10', timestamp: new Date(1700000000000),
    legacyResolution: { status: 'Completed', completedAt: '1700000001000', completedAtBlock: '1',
      completedAtTxHash: receipt.transactionHash } });
  return { ...native, receipt, transfer, requests: native.state.requests, clients: await native.clients('repair') };
}

test('repair CLI rejects unknown, duplicate and malformed flags and IDs', () => {
  assert.deepEqual(parseReconcileArgs(['--direction', 'vara-to-eth']), { ...options, limit: 1000 });
  assert.deepEqual(parseReconcileArgs(['--direction', 'eth-to-vara', '--limit', '1', '--after', 'b6b59062-10e4-4461-82d6-e18e35e163fa', '--apply']),
    { direction: 'eth-to-vara', limit: 1, after: 'b6b59062-10e4-4461-82d6-e18e35e163fa', apply: true });
  for (const args of [[], ['--direction', 'other'], ['--direction', 'vara-to-eth', '--limit', '0'],
    ['--direction', 'vara-to-eth', '--limit', '1001'], ['--direction', 'vara-to-eth', '--limit', '1e2'],
    ['--direction', 'vara-to-eth', '--after', 'garbage'], ['--direction', 'vara-to-eth', '--after', hash('aa') + '\n'],
    ['--direction', 'vara-to-eth', '--apply', '--apply'], ['--direction', 'vara-to-eth', '--unknown'],
    ['--direction', 'vara-to-eth', '--after'], ['--direction', 'vara-to-eth', '--limit', '01']]) {
    assert.throws(() => parseReconcileArgs(args));
  }
});

test('repair limit+1 advances over missing counterpart without writing in dry-run', async (t) => {
  const f = await fixture(t);
  delete f.transfer.legacyResolution;
  const store = storeOf(f.transfer, new Transfer({ ...f.transfer, id: hash('20'), nonce: '9' }),
    new Transfer({ ...f.transfer, id: hash('01'), status: Status.Failed }));
  const result = await reconcilePage(store, f.clients, options, queueAddress, managerAddress);
  assert.deepEqual(result, { ...options, lastExaminedId: f.transfer.id, hasMore: true, qualified: 0, pending: 1, heldIds: [] });
  assert.equal(store.queries.find((q) => q.entity === Transfer).opts.take, 2);
  const resumed = await reconcilePage(store, f.clients, { ...options, after: result.lastExaminedId }, queueAddress, managerAddress);
  assert.equal(resumed.lastExaminedId, hash('20'));
  assert.equal(resumed.hasMore, false);
  assert.equal(resumed.pending, 1);
  assert.equal(store.writes.length, 0);
});

test('direct original completion restores only on apply and retained bindings survive revalidation', async (t) => {
  const f = await fixture(t), store = storeOf(f.transfer);
  assert.equal((await reconcilePage(store, f.clients, options, queueAddress, managerAddress)).qualified, 1);
  assert.equal(store.writes.length, 0);
  assert.ok(f.requests.some(({ method }) => method === 'eth_getBlockReceipts'), 'native receipt inclusion must be proven');
  await reconcilePage(store, f.clients, { ...options, apply: true }, queueAddress, managerAddress);
  const original = structuredClone(store.rows.get(Transfer).get(f.transfer.id));
  assert.equal(original.status, Status.Completed);
  assert.equal(original.completedAtTxHash, f.receipt.transactionHash);
  assert.equal(original.completedAt.getTime(), 1700000001000);
  assert.equal(store.rows.get(CompletedTransfer).size, 1);
  assert.equal(store.rows.get(EthereumFinalityCheckpoint).size, 1);
  // A fresh higher finalized checkpoint must not rebind the original authorization.
  f.state.finalizedHeight = 3;
  await reconcilePage(store, f.clients, { ...options, apply: true }, queueAddress, managerAddress);
  assert.deepEqual(structuredClone(store.rows.get(Transfer).get(f.transfer.id)), original);
});

test('qualified original missing or changed effects fail the page with no writes', async (t) => {
  for (const fault of ['missing', 'corruptProof', 'effect', 'coordinates', 'binding', 'checkpoint', 'checkpointHeight', 'retainedReceipt']) {
    const f = await fixture(t, undefined, fault === 'checkpointHeight' ? 0 : 2), store = storeOf(f.transfer);
    const older = fault === 'checkpointHeight' ? await f.checkpoint(0, 'repair') : undefined;
    f.state.finalizedHeight = 2;
    await reconcilePage(store, f.clients, { ...options, apply: true }, queueAddress, managerAddress);
    store.writes.length = 0;
    if (fault === 'missing') await f.removeProof(f.receipt.transactionHash, 'repair');
    else if (fault === 'corruptProof') await f.corruptProof(f.receipt.transactionHash, 'repair');
    else if (fault === 'effect') store.rows.get(CompletedTransfer).get(`outbound:${queueAddress}:8`).receiptAuth.receipt.logs[0].data =
      manager.abi.encodeEventLog(manager.abi.getEvent('Bridged'), [hash('44'), addr('55'), addr('66'), 11n]).data;
    else if (fault === 'coordinates') store.rows.get(Transfer).get(f.transfer.id).completedAtBlock = 2n;
    else if (fault === 'checkpoint') store.rows.get(EthereumFinalityCheckpoint).clear();
    else if (fault === 'checkpointHeight') {
      const { checkpointId } = await import('../lib/eth/finality.js');
      const id = checkpointId(older);
      store.rows.get(EthereumFinalityCheckpoint).set(id, new EthereumFinalityCheckpoint({
        id, blockNumber: 0n, checkpoint: older }));
      store.rows.get(CompletedTransfer).get(`outbound:${queueAddress}:8`).receiptAuth.checkpointId = id;
      store.rows.get(Transfer).get(f.transfer.id).completionAuth.checkpointId = id;
    }
    else if (fault === 'retainedReceipt') delete store.rows.get(CompletedTransfer).get(`outbound:${queueAddress}:8`).outboundReceipt;
    else store.rows.get(Transfer).get(f.transfer.id).completionAuth.completionId = 'outbound:wrong';
    await assert.rejects(reconcilePage(store, f.clients, { ...options, apply: true }, queueAddress, managerAddress), /HOLD/);
    assert.equal(store.writes.length, 0);
  }
});

test('late page failure and final pin recheck precede all projection and checkpoint writes', async (t) => {
  const f = await fixture(t), second = new Transfer({ ...f.transfer, id: hash('20'), nonce: '9',
    legacyResolution: { ...f.transfer.legacyResolution, completedAtTxHash: hash('cc') } });
  const store = storeOf(f.transfer, second);
  await assert.rejects(reconcilePage(store, f.clients, { ...options, limit: 2, apply: true }, queueAddress, managerAddress), /HOLD/);
  assert.equal(store.writes.length, 0);
  assert.equal(store.rows.get(Transfer).get(f.transfer.id).status, Status.Bridging);
  let receiptProved = false;
  const g = await fixture(t), other = storeOf(g.transfer);
  g.state.fault = ({ side, method, result }) => {
    if (side === 'beacon' && receiptProved) throw new Error('final recheck outage');
    if (method === 'eth_getBlockReceipts') receiptProved = true;
    return result;
  };
  await assert.rejects(reconcilePage(other, g.clients, { ...options, apply: true }, queueAddress, managerAddress), /HOLD/);
  assert.equal(other.writes.length, 0);
});

async function inbound(t, outcome = 'Completed', { legacy = false, duplicate = false, wrongManager = false } = {}) {
  const deposit = { address: wrongManager ? addr('99') : managerAddress,
    ...manager.abi.encodeEventLog(manager.abi.getEvent('BridgingRequested'), [addr('55'), hash('44'), addr('66'), 10n]) };
  const prefix = { address: addr('99'), topics: [], data: '0x' };
  const specifications = legacy ? [
    { blockNumber: 1, transactionIndex: 0, logs: [prefix, prefix, prefix] },
    { blockNumber: 1, transactionIndex: 1, logs: [prefix, prefix, prefix] },
    { blockNumber: 1, transactionIndex: 2, logs: [{ ...deposit, address: addr('99') }, deposit, ...(duplicate ? [deposit] : [])] },
  ] : [{ blockNumber: 1, transactionIndex: 0, logs: [deposit] }];
  const f = await fixture(t, specifications);
  f.transfer = new Transfer({ id: 'b6b59062-10e4-4461-82d6-e18e35e163fa', txHash: f.receipt.transactionHash, blockNumber: 1n,
    sourceTransactionIndex: 0n, sourceLogIndex: 0n, sourceNetwork: Network.Ethereum, destNetwork: Network.Vara,
    source: addr('66'), destination: hash('77'), sender: addr('55'), receiver: hash('44'), amount: '10',
    status: Status.Bridging, nonce: 'source', timestamp: new Date(1700000001000),
    legacyResolution: { status: outcome, completedAt: '1700000002000', completedAtBlock: '300', completedAtTxHash: hash('dd') } });
  f.relay = new ReceiptRelay({ id: '200:0', blockNumber: 1n });
  f.settlement = new ReceiptSettlement({ id: '200:0:0', blockNumber: 300n, txHash: hash('dd'),
    timestamp: new Date(1700000002000), matched: false, deposit: { slot: '200', transaction_index: '0', log_index: '0',
      deposit_count: '1', operation_id: hash('a1'),
      eth_token_id: addr('66'), vara_token_id: hash('77'), sender: addr('55'), receiver: hash('44'), amount: '10',
      native: outcome === 'Refunded', ...(outcome === 'Refunded'
        ? { outcome, child: hash('a2'), payout_child: hash('a3'), returned_value: '10' } : {}) } });
  return f;
}

test('inbound repair keeps refund distinct, revalidates matched rows and preserves original bindings', async (t) => {
  for (const outcome of ['Completed', 'Refunded']) {
    const f = await inbound(t, outcome), store = storeOf(f.transfer, f.relay, f.settlement);
    const args = { ...options, direction: 'eth-to-vara' };
    assert.equal((await reconcilePage(store, f.clients, args, queueAddress, managerAddress)).qualified, 1);
    assert.equal(store.writes.length, 0);
    await reconcilePage(store, f.clients, { ...args, apply: true }, queueAddress, managerAddress);
    const saved = structuredClone(store.rows.get(Transfer).get(f.transfer.id));
    assert.equal(saved.status, outcome);
    assert.equal(saved.completedAtBlock, 300n);
    assert.equal(saved.completionAuth.receiptSettlementId, f.settlement.id);
    assert.equal(store.rows.get(ReceiptSettlement).get(f.settlement.id).matched, true);
    await reconcilePage(store, f.clients, { ...args, apply: true }, queueAddress, managerAddress);
    assert.deepEqual(structuredClone(store.rows.get(Transfer).get(f.transfer.id)), saved);
    assert.deepEqual(store.rows.get(ReceiptSettlement).get(f.settlement.id).deposit, f.settlement.deposit);
  }
});

test('inbound missing originals or changed terminal outcomes fail without replacing authorization', async (t) => {
  for (const fault of ['settlement', 'relay', 'source', 'outcome', 'coordinates']) {
    const f = await inbound(t, 'Refunded'), store = storeOf(f.transfer, f.relay, f.settlement);
    const args = { ...options, direction: 'eth-to-vara', apply: true };
    await reconcilePage(store, f.clients, args, queueAddress, managerAddress);
    store.writes.length = 0;
    if (fault === 'settlement') store.rows.get(ReceiptSettlement).clear();
    else if (fault === 'relay') store.rows.get(ReceiptRelay).clear();
    else if (fault === 'source') await f.removeProof(f.receipt.transactionHash, 'repair');
    else if (fault === 'outcome') delete store.rows.get(ReceiptSettlement).get(f.settlement.id).deposit.outcome;
    else store.rows.get(Transfer).get(f.transfer.id).completedAtTxHash = hash('fe');
    await assert.rejects(reconcilePage(store, f.clients, args, queueAddress, managerAddress), /HOLD/);
    assert.equal(store.writes.length, 0);
  }
});

test('inbound pending sources and missing counterparts advance only the external page hint', async (t) => {
  const f = await inbound(t), store = storeOf(f.transfer);
  const result = await reconcilePage(store, f.clients, { ...options, direction: 'eth-to-vara' }, queueAddress, managerAddress);
  assert.equal(result.lastExaminedId, f.transfer.id);
  assert.equal(result.pending, 1);
  assert.equal(result.qualified, 0);
  assert.equal(store.writes.length, 0);
});

test('repair refuses unmigrated terminal projections and receipt-less retained observations', async (t) => {
  for (const fault of ['unmigrated', 'observation']) {
    const f = await fixture(t);
    if (fault === 'unmigrated') f.transfer.status = Status.Completed;
    const rows = fault === 'observation' ? [new CompletedTransfer({ id: `outbound:${queueAddress}:8`,
      srcNetwork: Network.Vara, destNetwork: Network.Ethereum, txHash: f.receipt.transactionHash, blockNumber: 1n,
      timestamp: new Date(1700000001000), matched: false })] : [];
    const store = storeOf(f.transfer, ...rows);
    await assert.rejects(reconcilePage(store, f.clients, { ...options, apply: true }, queueAddress, managerAddress), /HOLD/);
    assert.equal(store.writes.length, 0);
  }
});

test('legacy observation JSON and timestamp survive independently reconstructed authorization', async (t) => {
  const f = await fixture(t), legacyReceipt = structuredClone(f.receipt);
  for (const field of ['transactionIndex', 'type', 'cumulativeGasUsed', 'logsBloom']) delete legacyReceipt[field];
  for (const log of legacyReceipt.logs) { delete log.transactionIndex; delete log.blockNumber; }
  const observation = new CompletedTransfer({ id: `outbound:${queueAddress}:8`,
    srcNetwork: Network.Vara, destNetwork: Network.Ethereum, txHash: f.receipt.transactionHash, blockNumber: 1n,
    timestamp: new Date(1), matched: false, legacyMatched: true,
    outboundReceipt: { receipt: legacyReceipt, logIndex: 1, nonce: '8', queue: queueAddress, manager: managerAddress } });
  const store = storeOf(f.transfer, observation);
  await reconcilePage(store, f.clients, { ...options, apply: true }, queueAddress, managerAddress);
  const saved = store.rows.get(CompletedTransfer).get(observation.id);
  assert.deepEqual(saved.outboundReceipt, observation.outboundReceipt);
  assert.equal(saved.timestamp.getTime(), 1);
  assert.equal(saved.receiptAuth.receipt.transactionIndex, '0x0');
  assert.equal(store.rows.get(Transfer).get(f.transfer.id).completedAt.getTime(), 1700000001000);
});

async function legacyInbound(t, outcome, settings) {
  const f = await inbound(t, outcome, { ...settings, legacy: true });
  f.transfer.sourceTransactionIndex = null;
  f.transfer.sourceLogIndex = null;
  f.relay.id = '200:2';
  f.settlement.id = '200:2:1';
  f.settlement.deposit.transaction_index = '2';
  f.settlement.deposit.log_index = '1';
  return f;
}

test('base-era inbound coordinates recover from the unique original receipt without dry-run mutation', async (t) => {
  for (const outcome of ['Completed', 'Refunded']) {
    for (const known of ['neither', 'transaction', 'log']) {
      const f = await legacyInbound(t, outcome);
      if (known === 'transaction') f.transfer.sourceTransactionIndex = 2n;
      if (known === 'log') f.transfer.sourceLogIndex = 1n;
      const store = storeOf(f.transfer, f.relay, f.settlement), before = store.snapshot();
      const args = { ...options, direction: 'eth-to-vara' };
      assert.equal((await reconcilePage(store, f.clients, args, queueAddress, managerAddress)).qualified, 1);
      assert.equal(store.writes.length, 0);
      assert.deepEqual(store.snapshot(), before);
      assert.equal(f.transfer.sourceTransactionIndex, known === 'transaction' ? 2n : null);
      assert.equal(f.transfer.sourceLogIndex, known === 'log' ? 1n : null);
      await reconcilePage(store, f.clients, { ...args, apply: true }, queueAddress, managerAddress);
      const saved = structuredClone(store.rows.get(Transfer).get(f.transfer.id));
      assert.equal(saved.status, outcome);
      assert.equal(saved.sourceTransactionIndex, 2n);
      assert.equal(saved.sourceLogIndex, 1n, 'use receipt-relative offset, never block-global logIndex 7');
      assert.equal(saved.sourceReceiptAuth.localLogIndex, 1);
      assert.equal(saved.completionAuth.receiptSettlementId, '200:2:1');
      await reconcilePage(store, f.clients, { ...args, apply: true }, queueAddress, managerAddress);
      assert.deepEqual(structuredClone(store.rows.get(Transfer).get(f.transfer.id)), saved);
    }
  }
});

test('authenticated coordinate backfill persists while settlement is pending and feeds the live tuple lookup', async (t) => {
  const f = await legacyInbound(t), store = storeOf(f.transfer, f.relay);
  const args = { ...options, direction: 'eth-to-vara', apply: true };
  const report = await reconcilePage(store, f.clients, args, queueAddress, managerAddress);
  assert.equal(report.pending, 1);
  const saved = structuredClone(store.rows.get(Transfer).get(f.transfer.id));
  assert.equal(saved.sourceTransactionIndex, 2n);
  assert.equal(saved.sourceLogIndex, 1n);
  assert.equal(saved.status, Status.Bridging);
  assert.ok(saved.sourceReceiptAuth);
  assert.equal(saved.completionAuth, undefined);
  await reconcilePage(store, f.clients, args, queueAddress, managerAddress);
  assert.deepEqual(structuredClone(store.rows.get(Transfer).get(f.transfer.id)), saved);
  // The Gear batch uses this exact lookup without the repair-only staged row.
  const { reconcileReceiptSettlements } = await import('../lib/gear/receipt-reconciliation.js');
  const { authenticateSourceReceipt } = await import('../lib/eth/completion.js');
  const { qualifyFinality } = await import('../lib/eth/finality.js');
  const checkpoint = await qualifyFinality(f.clients);
  const updates = await reconcileReceiptSettlements(store, [f.settlement],
    (row) => authenticateSourceReceipt(f.clients, row, managerAddress, checkpoint));
  assert.equal(updates.transfers[0].status, Status.Completed);
  assert.deepEqual(updates.transfers[0].sourceReceiptAuth, saved.sourceReceiptAuth);
});

test('coordinate recovery never guesses missing receipts, conflicting known indices or mismatched deposits', async (t) => {
  for (const fault of ['missing', 'transaction', 'log', 'manager', 'sender', 'receiver', 'source', 'amount']) {
    const f = await legacyInbound(t, 'Completed', { wrongManager: fault === 'manager' });
    if (fault === 'missing') f.state.fault = ({ method, result }) => method === 'eth_getBlockReceipts' ? null : result;
    else if (fault === 'transaction') f.transfer.sourceTransactionIndex = 0n;
    else if (fault === 'log') f.transfer.sourceLogIndex = 7n;
    else if (fault !== 'manager') f.transfer[fault] = fault === 'amount' ? '11' : fault === 'receiver' ? hash('99') : addr('99');
    const store = storeOf(f.transfer, f.relay, f.settlement), before = structuredClone(store.rows.get(Transfer).get(f.transfer.id));
    if (fault === 'missing') {
      await assert.rejects(reconcilePage(store, f.clients, { ...options, direction: 'eth-to-vara', apply: true },
        queueAddress, managerAddress), /HOLD/);
      assert.equal(store.writes.length, 0);
      assert.deepEqual(structuredClone(store.rows.get(Transfer).get(f.transfer.id)), before);
      continue;
    }
    const result = await reconcilePage(store, f.clients, { ...options, direction: 'eth-to-vara', apply: true }, queueAddress, managerAddress);
    assert.equal(result.qualified, 0, fault);
    assert.equal(result.pending, 1, fault);
    assert.deepEqual(structuredClone(store.rows.get(Transfer).get(f.transfer.id)), before, fault);
    assert.equal(store.rows.get(ReceiptSettlement).get(f.settlement.id).matched, false);
  }
});

test('ambiguous receipt deposits and recovered transfer tuples HOLD without writes', async (t) => {
  for (const fault of ['deposit', 'storedTuple', 'pageTuple']) {
    const f = await legacyInbound(t, 'Completed', { duplicate: fault === 'deposit' });
    const duplicate = new Transfer({ ...f.transfer, id: 'c6b59062-10e4-4461-82d6-e18e35e163fa' });
    if (fault === 'storedTuple') { duplicate.sourceTransactionIndex = 2n; duplicate.sourceLogIndex = 1n; }
    const store = storeOf(f.transfer, f.relay, f.settlement, ...(fault === 'deposit' ? [] : [duplicate]));
    const before = store.snapshot();
    await assert.rejects(reconcilePage(store, f.clients,
      { ...options, direction: 'eth-to-vara', limit: 2, apply: true }, queueAddress, managerAddress), /HOLD: ambiguous/);
    assert.equal(store.writes.length, 0);
    assert.deepEqual(store.snapshot(), before);
  }
});

test('coordinate recovery is staged until the complete page and final checkpoint succeed', async (t) => {
  for (const fault of ['laterTransfer', 'finalCheckpoint']) {
    const f = await legacyInbound(t), second = new Transfer({ ...f.transfer, id: 'c6b59062-10e4-4461-82d6-e18e35e163fa', txHash: hash('cc') });
    const store = storeOf(f.transfer, f.relay, f.settlement, ...(fault === 'laterTransfer' ? [second] : []));
    const before = store.snapshot();
    let receiptProved = false;
    if (fault === 'finalCheckpoint') f.state.fault = ({ side, method, result }) => {
      if (side === 'beacon' && receiptProved) throw new Error('later finality unavailable');
      if (method === 'eth_getBlockReceipts') receiptProved = true;
      return result;
    };
    await assert.rejects(reconcilePage(store, f.clients,
      { ...options, direction: 'eth-to-vara', limit: 2, apply: true }, queueAddress, managerAddress), /HOLD/);
    assert.equal(store.writes.length, 0);
    assert.deepEqual(store.snapshot(), before);
  }
});

test('missing or conflicting coordinates on previously authenticated sources cannot be repaired', async (t) => {
  for (const terminal of [false, true]) {
    for (const coordinate of ['sourceTransactionIndex', 'sourceLogIndex']) {
      for (const value of [null, 9n]) {
        const f = await legacyInbound(t), store = storeOf(f.transfer, f.relay, ...(terminal ? [f.settlement] : []));
        const args = { ...options, direction: 'eth-to-vara', apply: true };
        await reconcilePage(store, f.clients, args, queueAddress, managerAddress);
        store.rows.get(Transfer).get(f.transfer.id)[coordinate] = value;
        const before = store.snapshot();
        store.writes.length = 0;
        await assert.rejects(reconcilePage(store, f.clients, args, queueAddress, managerAddress), /HOLD/);
        assert.equal(store.writes.length, 0);
        assert.deepEqual(store.snapshot(), before);
      }
    }
  }
});
