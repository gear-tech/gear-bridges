import assert from 'node:assert/strict';
import { test } from 'node:test';
import { fileURLToPath } from 'node:url';
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
function fixture() {
  const headers = [0, 1, 2].map((number) => ({ number: `0x${number}`, hash: hash(['a0', 'a1', 'a2'][number]),
    parentHash: hash(number ? ['a0', 'a1'][number - 1] : '00'), timestamp: `0x${(1700000000 + number).toString(16)}`,
    stateRoot: hash('01'), transactionsRoot: hash('02'), receiptsRoot: hash('03'), transactions: number === 1 ? [hash('bb')] : [] }));
  const log = (abi, name, args, address, index) => ({ ...abi.encodeEventLog(abi.getEvent(name), args),
    address, logIndex: `0x${index}`, transactionIndex: '0x0', blockNumber: '0x1', transactionHash: hash('bb'), blockHash: headers[1].hash, removed: false });
  const receipt = { transactionHash: hash('bb'), blockHash: headers[1].hash, blockNumber: '0x1', transactionIndex: '0x0',
    status: '0x1', type: '0x2', cumulativeGasUsed: '0x5208', logsBloom: `0x${'00'.repeat(256)}`,
    logs: [log(manager.abi, 'Bridged', [hash('44'), addr('55'), addr('66'), 10n], managerAddress, 0),
      log(queue.abi, 'MessageProcessed', [9n, hash('77'), 8n, managerAddress], queueAddress, 1)] };
  const transfer = new Transfer({ id: hash('10'), nonce: '8', txHash: hash('ee'), blockNumber: 9n,
    sourceNetwork: Network.Vara, destNetwork: Network.Ethereum, status: Status.Bridging,
    bridgingStartedAtBlock: 9n, ethBridgeBuiltInMsgHash: hash('77'), sender: hash('44'), receiver: addr('55'),
    source: hash('66'), destination: addr('66'), amount: '10', timestamp: new Date(1700000000000),
    legacyResolution: { status: 'Completed', completedAt: '1700000001000', completedAtBlock: '1', completedAtTxHash: hash('bb') } });
  const requests = [];
  const call = async (method, params) => {
    requests.push([method, params]);
    if (method === 'eth_chainId') return '0x1';
    if (method === 'eth_getBlockByNumber') return structuredClone(headers[params[0] === 'finalized' ? 2 : Number(BigInt(params[0]))]);
    if (method === 'eth_getBlockByHash') return structuredClone(headers.find((h) => h.hash === params[0]));
    if (method === 'eth_getTransactionReceipt') return params[0] === receipt.transactionHash ? structuredClone(receipt) : null;
    throw new Error(`Unexpected RPC ${method}`);
  };
  return { receipt, transfer, headers, requests, clients: { primaryCall: call, witnessCall: call, chainId: 1n, genesisHash: headers[0].hash } };
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

test('repair limit+1 advances over missing counterpart without writing in dry-run', async () => {
  const f = fixture();
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

test('direct original completion restores only on apply and retained bindings survive revalidation', async () => {
  const f = fixture(), store = storeOf(f.transfer);
  assert.equal((await reconcilePage(store, f.clients, options, queueAddress, managerAddress)).qualified, 1);
  assert.equal(store.writes.length, 0);
  assert.ok(f.requests.filter(([method]) => method === 'eth_getTransactionReceipt').every(([, params]) => params[0] === hash('bb')));
  await reconcilePage(store, f.clients, { ...options, apply: true }, queueAddress, managerAddress);
  const original = structuredClone(store.rows.get(Transfer).get(f.transfer.id));
  assert.equal(original.status, Status.Completed);
  assert.equal(original.completedAtTxHash, hash('bb'));
  assert.equal(original.completedAt.getTime(), 1700000001000);
  assert.equal(store.rows.get(CompletedTransfer).size, 1);
  assert.equal(store.rows.get(EthereumFinalityCheckpoint).size, 1);
  // A fresh higher finalized checkpoint must not rebind the original authorization.
  f.headers.push({ ...f.headers[2], number: '0x3', hash: hash('a3'), parentHash: hash('a2') });
  const primary = f.clients.primaryCall;
  f.clients.primaryCall = f.clients.witnessCall = (method, params) => method === 'eth_getBlockByNumber' && params[0] === 'finalized'
    ? Promise.resolve(structuredClone(f.headers[3])) : primary(method, params);
  await reconcilePage(store, f.clients, { ...options, apply: true }, queueAddress, managerAddress);
  assert.deepEqual(structuredClone(store.rows.get(Transfer).get(f.transfer.id)), original);
});

test('qualified original missing or changed effects fail the page with no writes', async () => {
  for (const fault of ['missing', 'effect', 'coordinates', 'binding', 'checkpoint', 'checkpointHeight', 'retainedReceipt']) {
    const f = fixture(), store = storeOf(f.transfer);
    await reconcilePage(store, f.clients, { ...options, apply: true }, queueAddress, managerAddress);
    store.writes.length = 0;
    if (fault === 'missing') {
      const call = f.clients.primaryCall;
      f.clients.primaryCall = f.clients.witnessCall = (m, p) => m === 'eth_getTransactionReceipt' ? Promise.resolve(null) : call(m, p);
    } else if (fault === 'effect') f.receipt.logs[0].data = manager.abi.encodeEventLog(manager.abi.getEvent('Bridged'), [hash('44'), addr('55'), addr('66'), 11n]).data;
    else if (fault === 'coordinates') store.rows.get(Transfer).get(f.transfer.id).completedAtBlock = 2n;
    else if (fault === 'checkpoint') store.rows.get(EthereumFinalityCheckpoint).clear();
    else if (fault === 'checkpointHeight') {
      const retained = [...store.rows.get(EthereumFinalityCheckpoint).values()][0];
      const id = `1:${f.headers[0].hash}:0`;
      store.rows.get(EthereumFinalityCheckpoint).set(id, new EthereumFinalityCheckpoint({ id, blockNumber: 0n,
        checkpoint: { ...retained.checkpoint, number: '0', hash: f.headers[0].hash, parentHash: hash('00'), timestamp: '1700000000' } }));
      store.rows.get(CompletedTransfer).get(`outbound:${queueAddress}:8`).receiptAuth.checkpointId = id;
      store.rows.get(Transfer).get(f.transfer.id).completionAuth.checkpointId = id;
    }
    else if (fault === 'retainedReceipt') delete store.rows.get(CompletedTransfer).get(`outbound:${queueAddress}:8`).outboundReceipt;
    else store.rows.get(Transfer).get(f.transfer.id).completionAuth.completionId = 'outbound:wrong';
    await assert.rejects(reconcilePage(store, f.clients, { ...options, apply: true }, queueAddress, managerAddress), /HOLD/);
    assert.equal(store.writes.length, 0);
  }
});

test('late page failure and final pin recheck precede all projection and checkpoint writes', async () => {
  const f = fixture(), second = new Transfer({ ...f.transfer, id: hash('20'), nonce: '9',
    legacyResolution: { ...f.transfer.legacyResolution, completedAtTxHash: hash('cc') } });
  const store = storeOf(f.transfer, second), call = f.clients.witnessCall;
  f.clients.witnessCall = (m, p) => m === 'eth_getTransactionReceipt' && p[0] === hash('cc')
    ? Promise.reject(new Error('HOLD: witness unavailable')) : call(m, p);
  await assert.rejects(reconcilePage(store, f.clients, { ...options, limit: 2, apply: true }, queueAddress, managerAddress), /HOLD/);
  assert.equal(store.writes.length, 0);
  assert.equal(store.rows.get(Transfer).get(f.transfer.id).status, Status.Bridging);
  let finalizedCalls = 0;
  const g = fixture(), other = storeOf(g.transfer), original = g.clients.witnessCall;
  g.clients.witnessCall = (m, p) => {
    if (m === 'eth_getBlockByNumber' && p[0] === 'finalized' && ++finalizedCalls > 1) throw new Error('HOLD: final recheck outage');
    return original(m, p);
  };
  await assert.rejects(reconcilePage(other, g.clients, { ...options, apply: true }, queueAddress, managerAddress), /HOLD/);
  assert.equal(other.writes.length, 0);
});

function inbound(outcome = 'Completed') {
  const f = fixture();
  f.receipt.logs = [{ ...f.receipt.logs[0], address: managerAddress,
    ...manager.abi.encodeEventLog(manager.abi.getEvent('BridgingRequested'), [addr('55'), hash('44'), addr('66'), 10n]) }];
  f.transfer = new Transfer({ id: 'b6b59062-10e4-4461-82d6-e18e35e163fa', txHash: hash('bb'), blockNumber: 1n,
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

test('inbound repair keeps refund distinct, revalidates matched rows and preserves original bindings', async () => {
  for (const outcome of ['Completed', 'Refunded']) {
    const f = inbound(outcome), store = storeOf(f.transfer, f.relay, f.settlement);
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

test('inbound missing originals or changed terminal outcomes fail without replacing authorization', async () => {
  for (const fault of ['settlement', 'relay', 'source', 'outcome', 'coordinates']) {
    const f = inbound('Refunded'), store = storeOf(f.transfer, f.relay, f.settlement);
    const args = { ...options, direction: 'eth-to-vara', apply: true };
    await reconcilePage(store, f.clients, args, queueAddress, managerAddress);
    store.writes.length = 0;
    if (fault === 'settlement') store.rows.get(ReceiptSettlement).clear();
    else if (fault === 'relay') store.rows.get(ReceiptRelay).clear();
    else if (fault === 'source') {
      const call = f.clients.primaryCall;
      f.clients.primaryCall = f.clients.witnessCall = (m, p) => m === 'eth_getTransactionReceipt' ? Promise.resolve(null) : call(m, p);
    } else if (fault === 'outcome') delete store.rows.get(ReceiptSettlement).get(f.settlement.id).deposit.outcome;
    else store.rows.get(Transfer).get(f.transfer.id).completedAtTxHash = hash('fe');
    await assert.rejects(reconcilePage(store, f.clients, args, queueAddress, managerAddress), /HOLD/);
    assert.equal(store.writes.length, 0);
  }
});

test('inbound pending sources and missing counterparts advance only the external page hint', async () => {
  const f = inbound(), store = storeOf(f.transfer);
  const result = await reconcilePage(store, f.clients, { ...options, direction: 'eth-to-vara' }, queueAddress, managerAddress);
  assert.equal(result.lastExaminedId, f.transfer.id);
  assert.equal(result.pending, 1);
  assert.equal(result.qualified, 0);
  assert.equal(store.writes.length, 0);
});

test('repair refuses unmigrated terminal projections and receipt-less retained observations', async () => {
  for (const fault of ['unmigrated', 'observation']) {
    const f = fixture();
    if (fault === 'unmigrated') f.transfer.status = Status.Completed;
    const rows = fault === 'observation' ? [new CompletedTransfer({ id: `outbound:${queueAddress}:8`,
      srcNetwork: Network.Vara, destNetwork: Network.Ethereum, txHash: hash('bb'), blockNumber: 1n,
      timestamp: new Date(1700000001000), matched: false })] : [];
    const store = storeOf(f.transfer, ...rows);
    await assert.rejects(reconcilePage(store, f.clients, { ...options, apply: true }, queueAddress, managerAddress), /HOLD/);
    assert.equal(store.writes.length, 0);
  }
});

test('legacy observation JSON and timestamp survive independently reconstructed authorization', async () => {
  const f = fixture(), legacyReceipt = structuredClone(f.receipt);
  for (const field of ['transactionIndex', 'type', 'cumulativeGasUsed', 'logsBloom']) delete legacyReceipt[field];
  for (const log of legacyReceipt.logs) { delete log.transactionIndex; delete log.blockNumber; }
  const observation = new CompletedTransfer({ id: `outbound:${queueAddress}:8`,
    srcNetwork: Network.Vara, destNetwork: Network.Ethereum, txHash: hash('bb'), blockNumber: 1n,
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
