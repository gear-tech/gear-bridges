import assert from 'node:assert/strict';
import { test, beforeEach, after } from 'node:test';
import { fileURLToPath } from 'node:url';
import { syncBuiltinESMExports } from 'node:module';
import { spawnSync } from 'node:child_process';
import { createServer } from 'node:http';

// Real loopback transports exercise ethers without weakening production qualification.
let primaryRpc, witnessRpc;
async function serveRpc(dispatch) {
  const server = createServer(async (request, response) => {
    try {
      let body = '';
      for await (const chunk of request) body += chunk;
      const execute = async ({ id, method, params }) => {
        try { return { jsonrpc: '2.0', id, result: await dispatch(method, params) }; }
        catch (error) { return { jsonrpc: '2.0', id, error: { code: -32000, message: error.message } }; }
      };
      const input = JSON.parse(body);
      response.setHeader('content-type', 'application/json');
      response.end(JSON.stringify(Array.isArray(input) ? await Promise.all(input.map(execute)) : await execute(input)));
    } catch (error) { response.writeHead(500).end(error.message); }
  });
  await new Promise((resolve) => server.listen(0, '127.0.0.1', resolve));
  after(() => new Promise((resolve) => { server.close(resolve); server.closeAllConnections(); }));
  return `http://127.0.0.1:${server.address().port}`;
}
const primaryUrl = await serveRpc((method, params) => primaryRpc(method, params));
const witnessUrl = await serveRpc((method, params) => witnessRpc(method, params));

const addr = (byte) => `0x${byte.repeat(20)}`;
const hash = (byte) => `0x${byte.repeat(32)}`;
Object.assign(process.env, {
  ETH_API_PATH: fileURLToPath(new URL('../../../api/ethereum', import.meta.url)),
  GEAR_API_PATH: fileURLToPath(new URL('../../../api/gear', import.meta.url)),
  ETH_RPC_URL: primaryUrl, ETH_WITNESS_RPC: witnessUrl, ETH_CHAIN_ID: '1', ETH_GENESIS_HASH: hash('00'),
  ETH_ERC20_MANAGER: addr('11'), ETH_MSQ_QUEUE: addr('22'),
  ETH_BRIDGING_PAYMENT: addr('33'), ETH_SQUID_API_KEY: 'local-test',
  GEAR_VFT_MANAGER: hash('11'), GEAR_HISTORICAL_PROXY: hash('22'), GEAR_BRIDGING_PAYMENT: hash('33'),
  GEAR_CHECKPOINT_CLIENT: hash('44'), GEAR_SQUID_API_KEY: 'local-test', ETH_HTTP_RPC_URL: 'http://127.0.0.1:1',
});
const { handleBatch } = await import('../lib/eth/handler.js');
const manager = await import('../lib/eth/abi/erc20-manager.js');
const queue = await import('../lib/eth/abi/message-queue.js');
const payment = await import('../lib/eth/abi/bridging-payment.js');
const { BatchState: GearState } = await import('../lib/gear/batch-state.js');
const { requireFinalizedBatch: requireEthFinality, createEthereumEvidenceClients, qualifyFinality,
  createReceiptAuthenticator } = await import('../lib/eth/finality.js');
const { authenticateSourceReceipt } = await import('../lib/eth/completion.js');
const { reconcileReceiptSettlements } = await import('../lib/gear/receipt-reconciliation.js');
const { requireFinalizedBatch: requireGearFinality } = await import('../lib/gear/finality.js');
const { Transfer, CompletedTransfer, MerkleRootInMessageQueue, EthereumFinalityCheckpoint, Pair, Network, Status, ReceiptRelay, ReceiptSettlement } = await import('../lib/model/index.js');
const quiet = { child() { return this; }, info() {}, debug() {}, warn() {}, error() {} };
const block = { height: 11, hash: hash('aa'), timestamp: 1_700_000_000_000 };
function memoryStore() {
  const rows = new Map(), queries = [], saves = [];
  const matches = (row, where) => !where || (Array.isArray(where)
    ? where.some((part) => matches(row, part))
    : Object.entries(where).every(([key, value]) => {
      const compare = (expected) => {
        if (expected?.type === 'in') return expected.value.includes(row[key]);
        if (expected?.type === 'moreThan') return row[key] > expected.value;
        if (expected?.type === 'isNull') return row[key] == null;
        if (expected?.type === 'not') return !compare(expected.child ?? expected.value);
        return row[key] === expected;
      };
      return compare(value);
    }));
  return { rows, queries, saves,
    async find(entity, options) {
      queries.push({ entity, options });
      let result = [...(rows.get(entity)?.values() ?? [])].filter((row) => matches(row, options?.where));
      if (options?.order?.id) result.sort((a, b) => a.id < b.id ? -1 : a.id > b.id ? 1 : 0);
      if (options?.take != null) result = result.slice(0, options.take);
      return result;
    },
    async findOneBy(entity, where) {
      assert.equal(entity, EthereumFinalityCheckpoint, 'receipt reconciliation must keep batched joins');
      return [...(rows.get(entity)?.values() ?? [])].find((row) => matches(row, where));
    },
    async save(items) {
      saves.push(items);
      for (const row of Array.isArray(items) ? items : [items]) {
        if (!rows.has(row.constructor)) rows.set(row.constructor, new Map());
        rows.get(row.constructor).set(row.id, row);
      }
    },
    async insert(items) {
      for (const row of Array.isArray(items) ? items : [items]) {
        assert.equal(rows.get(row.constructor)?.has(row.id) ?? false, false, 'checkpoint insertion must not overwrite');
        if (!rows.has(row.constructor)) rows.set(row.constructor, new Map());
        rows.get(row.constructor).set(row.id, row);
      }
    },
    async remove() {},
  };
}


const txHash = hash('bb');
function event(abi, name, args, address, index) {
  return { ...abi.encodeEventLog(abi.getEvent(name), args), address, logIndex: `0x${index.toString(16)}`,
    transactionHash: txHash, blockHash: block.hash, blockNumber: '0xb', transactionIndex: '0x0' };
}
function outbound(effect = [hash('44'), addr('55'), addr('66'), 10n]) {
  const logs = [event(manager.abi, 'Bridged', effect, addr('11'), 0),
    event(queue.abi, 'MessageProcessed', [9n, hash('77'), 8n, addr('11')], addr('22'), 1)];
  const receipt = fullReceipt(logs);
  const transfer = new Transfer({ id: 'original', nonce: '8', sourceNetwork: Network.Vara,
    status: Status.Bridging, bridgingStartedAtBlock: 9n, ethBridgeBuiltInMsgHash: hash('77'),
    sender: hash('44'), receiver: addr('55'), destination: addr('66'), amount: '10' });
  return { receipt, transfer };
}
function fullReceipt(logs, height = 11, transactionIndex = 0, transactionHash = txHash) {
  return { transactionHash, blockHash: header(height).hash, blockNumber: `0x${height.toString(16)}`,
    transactionIndex: `0x${transactionIndex.toString(16)}`, status: '0x1', type: '0x2',
    cumulativeGasUsed: '0x5208', logsBloom: `0x${'00'.repeat(256)}`, logs };
}
function header(height) {
  const identity = (n) => n === 11 ? block.hash : `0x${n.toString(16).padStart(64, '0')}`;
  return { number: `0x${height.toString(16)}`, hash: identity(height),
    parentHash: identity(Math.max(0, height - 1)), stateRoot: hash('01'),
    transactionsRoot: hash('02'), receiptsRoot: hash('03'), timestamp: '0x6553f100' };
}
function evidenceRpc(receipts, finalizedHeight = 11, canonicalHash) {
  return async (method, params) => {
    if (method === 'eth_chainId') return '0x1';
    if (method === 'eth_getTransactionReceipt') return receipts.find((r) => r.transactionHash === params[0]) ?? null;
    if (method === 'eth_getBlockByNumber') {
      const height = params[0] === 'finalized' ? finalizedHeight : Number(BigInt(params[0]));
      const result = header(height);
      if (height === 11 && canonicalHash) result.hash = canonicalHash;
      result.transactions = [];
      for (const receipt of receipts.filter((r) => BigInt(r.blockNumber) === BigInt(height))) {
        const index = Number(BigInt(receipt.transactionIndex));
        while (result.transactions.length < index) result.transactions.push(hash('ff'));
        result.transactions[index] = receipt.transactionHash;
      }
      return result;
    }
    throw new Error(`Unexpected RPC method ${method}`);
  };
}
function context(receipt, transfer, canonicalHash = block.hash, finalizedHeight = '0xb') {
  const store = memoryStore();
  store.findOneBy = async (entity, where) => (await store.find(entity, { where }))[0];
  if (transfer) store.rows.set(Transfer, new Map([[transfer.id, transfer]]));
  primaryRpc = evidenceRpc([receipt], Number(BigInt(finalizedHeight)), canonicalHash);
  witnessRpc = evidenceRpc([receipt], Number(BigInt(finalizedHeight)));
  const rpc = { call: (method, params) => primaryRpc(method, params),
    batchCall: (calls) => Promise.all(calls.map(({ method, params }) => primaryRpc(method, params))) };
  return { _chain: { client: rpc }, log: quiet, store,
    get saved() { return store.saves.flat(); },
    blocks: [{ header: block, logs: [receipt.logs.at(-1)].map((l) => ({ ...l,
      logIndex: Number(BigInt(l.logIndex)), transactionIndex: Number(BigInt(l.transactionIndex)) })) }],
  };
}
function stored(store, original) { return store.rows.get(original.constructor)?.get(original.id) ?? original; }
function inboundReceipt(height = 100, transactionHash = hash('bc')) {
  const logs = Array.from({ length: 6 }, (_, index) => ({
    ...event(manager.abi, 'BridgingRequested', [addr('55'), hash('44'), addr('66'), 10n], addr('11'), index + 20),
    transactionHash, blockHash: header(height).hash, blockNumber: `0x${height.toString(16)}`, transactionIndex: '0x2',
  }));
  return fullReceipt(logs, height, 2, transactionHash);
}
beforeEach(() => {
  primaryRpc = evidenceRpc([inboundReceipt(), inboundReceipt(101, hash('bd'))], 101);
  witnessRpc = primaryRpc;
});
test('matching finalized headers cannot authorize different primary and witness receipts', async () => {
  const { receipt, transfer } = outbound();
  const ctx = context(receipt, transfer);
  const witnessReceipt = structuredClone(receipt);
  witnessReceipt.logs[0] = { ...witnessReceipt.logs[0],
    ...manager.abi.encodeEventLog(manager.abi.getEvent('Bridged'), [hash('44'), addr('55'), addr('66'), 11n]) };
  primaryRpc = evidenceRpc([receipt]);
  witnessRpc = evidenceRpc([witnessReceipt]);
  await assert.rejects(handleBatch(ctx), /HOLD:/);
  assert.equal(transfer.status, Status.Bridging);
  assert.equal(transfer.completedAt, undefined);
  assert.equal(ctx.saved.length, 0);
});

test('malformed original log coordinates HOLD rather than becoming unmatched success', async () => {
  for (const mutate of [
    (receipt) => { receipt.logs[0].transactionHash = hash('cc'); },
    (receipt) => { receipt.logs[0].blockNumber = '0xa'; },
    (receipt) => { receipt.logs[0].transactionIndex = '0x1'; },
    (receipt) => { receipt.logs[0].removed = true; },
    (receipt) => { receipt.logs[1].logIndex = '0x0'; },
  ]) {
    const { receipt, transfer } = outbound();
    mutate(receipt);
    const ctx = context(receipt, transfer);
    await assert.rejects(handleBatch(ctx), /HOLD:/);
    assert.equal(transfer.status, Status.Bridging);
    assert.equal(ctx.saved.length, 0);
  }
});

test('deferred completion rereads both receipts, not merely their matching header', async () => {
  const { receipt, transfer } = outbound();
  const ctx = context(receipt);
  await handleBatch(ctx);
  const retained = [...ctx.store.rows.get(CompletedTransfer).values()][0];
  const original = structuredClone(retained);
  await ctx.store.save(transfer);
  ctx.blocks = [];
  const forged = structuredClone(receipt);
  forged.logs[0] = { ...forged.logs[0],
    ...manager.abi.encodeEventLog(manager.abi.getEvent('Bridged'), [hash('44'), addr('55'), addr('66'), 11n]) };
  witnessRpc = evidenceRpc([forged]);
  ctx.store.saves.length = 0;
  await assert.rejects(handleBatch(ctx), /HOLD:/);
  assert.equal(stored(ctx.store, transfer).status, Status.Bridging);
  assert.equal(stored(ctx.store, transfer).completedAt, undefined);
  assert.deepEqual(structuredClone(stored(ctx.store, retained)), original);
  assert.equal(ctx.saved.length, 0);
});

for (const legacyMatched of [true, false]) {
  test(`legacy backlog with legacyMatched=${legacyMatched} cannot wedge fresh live completion`, async () => {
    const { receipt, transfer } = outbound();
    const ctx = context(receipt, transfer);
    const oldReceipt = structuredClone(receipt);
    oldReceipt.transactionHash = hash('ce');
    for (const log of oldReceipt.logs) log.transactionHash = oldReceipt.transactionHash;
    const legacy = new CompletedTransfer({ id: `outbound:${addr('22')}:7`, srcNetwork: Network.Vara,
      destNetwork: Network.Ethereum, txHash: oldReceipt.transactionHash, blockNumber: 11n,
      timestamp: new Date(block.timestamp), matched: false, legacyMatched, receiptAuth: null,
      outboundReceipt: { receipt: oldReceipt, logIndex: 1, nonce: '7', queue: addr('22'), manager: addr('11') } });
    const oldTransfer = new Transfer({ ...transfer, id: 'legacy-original', nonce: '7',
      completedAt: null, completedAtBlock: null, completedAtTxHash: null, completionAuth: null,
      legacyResolution: { status: 'Completed', completedAt: String(block.timestamp),
        completedAtBlock: '11', completedAtTxHash: oldReceipt.transactionHash } });
    await ctx.store.save([legacy, oldTransfer]);
    const savedLegacy = structuredClone(legacy), savedTransfer = structuredClone(oldTransfer);
    const requests = [];
    const dispatch = evidenceRpc([receipt]);
    primaryRpc = witnessRpc = async (method, params) => {
      if (method === 'eth_getTransactionReceipt') requests.push(params[0]);
      return dispatch(method, params);
    };
    ctx.store.saves.length = 0;
    await handleBatch(ctx);
    assert.equal(stored(ctx.store, transfer).status, Status.Completed, 'fresh qualified work must commit');
    assert.deepEqual(structuredClone(stored(ctx.store, legacy)), savedLegacy, 'retain original legacy JSON and flags');
    assert.deepEqual(structuredClone(stored(ctx.store, oldTransfer)), savedTransfer, 'legacy claim remains nonterminal');
    assert.ok(!requests.includes(oldReceipt.transactionHash), 'standalone repair owns unauthenticated legacy history');
  });
}

test('legacy backlog cannot consume the live pending page ahead of qualified counterpart-lag work', async () => {
  const { receipt, transfer } = outbound();
  const ctx = context(receipt);
  await handleBatch(ctx);
  const qualified = [...ctx.store.rows.get(CompletedTransfer).values()][0];
  await ctx.store.save(transfer);
  await ctx.store.save(Array.from({ length: 1000 }, (_, index) => new CompletedTransfer({
    ...qualified, id: `legacy-${String(index).padStart(4, '0')}`, receiptAuth: null,
    legacyMatched: index % 2 === 0, txHash: hash('ce'),
    outboundReceipt: { ...qualified.outboundReceipt, nonce: String(1000 + index),
      receipt: { ...receipt, transactionHash: hash('ce') } },
  })));
  ctx.blocks = [];
  const { handleBatch: restarted } = await import('../lib/eth/handler.js?legacy-backlog-page');
  await restarted(ctx);
  assert.equal(stored(ctx.store, transfer).status, Status.Completed);
  assert.equal(stored(ctx.store, qualified).matched, true);
  assert.equal([...ctx.store.rows.get(CompletedTransfer).values()].filter((row) => row.legacyMatched != null)
    .every((row) => !row.matched && row.receiptAuth == null), true);
});

test('legacy backlog exemption never skips qualified, unmarked or current-batch evidence', async () => {
  for (const mode of ['qualified', 'unmarked', 'current']) {
    const { receipt, transfer } = outbound();
    const ctx = context(receipt);
    await handleBatch(ctx);
    const retained = [...ctx.store.rows.get(CompletedTransfer).values()][0];
    retained.legacyMatched = mode === 'unmarked' ? null : false;
    if (mode !== 'qualified') retained.receiptAuth = null;
    await ctx.store.save(transfer);
    if (mode !== 'current') ctx.blocks = [];
    const original = structuredClone(retained);
    primaryRpc = witnessRpc = evidenceRpc([]);
    ctx.store.saves.length = 0;
    await assert.rejects(handleBatch(ctx), /HOLD:.*receipt/i, mode);
    assert.equal(stored(ctx.store, transfer).status, Status.Bridging);
    assert.deepEqual(structuredClone(stored(ctx.store, retained)), original);
    assert.equal(ctx.saved.length, 0);
  }
});

test('every consumed manager, queue and payment event must occur in the qualified receipt', async () => {
  const events = [
    event(manager.abi, 'BridgingRequested', [addr('55'), hash('44'), addr('66'), 10n], addr('11'), 2),
    event(queue.abi, 'MessageProcessed', [9n, hash('77'), 8n, addr('11')], addr('22'), 2),
    event(queue.abi, 'MerkleRoot', [12n, hash('88')], addr('22'), 2),
    event(payment.abi, 'FeePaid', [], addr('33'), 2),
  ];
  for (const consumed of events) {
    const { receipt, transfer } = outbound();
    const ctx = context(receipt, transfer);
    ctx.blocks[0].logs = [{ ...consumed, logIndex: 2, transactionIndex: 0 }];
    await assert.rejects(handleBatch(ctx), /HOLD:/);
    assert.equal(transfer.status, Status.Bridging);
    assert.equal(ctx.saved.length, 0);
  }
});

test('inbound creation uses qualified receipt-local coordinates and header time for every deposit', async () => {
  const deposits = [0, 1].map((index) =>
    event(manager.abi, 'BridgingRequested', [addr('55'), hash('44'), addr('66'), 10n], addr('11'), 20 + index));
  const fee = event(payment.abi, 'FeePaid', [], addr('33'), 22);
  const receipt = fullReceipt([...deposits, fee]);
  const ctx = context(receipt);
  ctx.store.rows.set(Pair, new Map([['pair', new Pair({ id: 'pair', varaToken: hash('77'), ethToken: addr('66'), isRemoved: false })]]));
  ctx.blocks[0] = { header: { ...block, timestamp: 0 }, logs: receipt.logs.map((log) => ({
    ...log, logIndex: Number(BigInt(log.logIndex)), transactionIndex: 0,
  })) };
  let primaryReceipts = 0, witnessReceipts = 0;
  const canonical = evidenceRpc([receipt]);
  primaryRpc = async (method, params) => {
    if (method === 'eth_getTransactionReceipt') primaryReceipts++;
    return canonical(method, params);
  };
  witnessRpc = async (method, params) => {
    if (method === 'eth_getTransactionReceipt') witnessReceipts++;
    return canonical(method, params);
  };
  await handleBatch(ctx);
  const created = [...ctx.store.rows.get(Transfer).values()];
  assert.equal(created.length, 2);
  assert.notEqual(created[0].nonce, created[1].nonce);
  assert.deepEqual(created.map((row) => row.sourceLogIndex), [0n, 1n]);
  for (const row of created) {
    assert.equal(row.txHash, txHash);
    assert.equal(row.sourceTransactionIndex, 0n);
    assert.equal(row.timestamp.getTime(), block.timestamp);
    assert.equal(row.status, Status.Bridging);
    assert.equal(row.sourceReceiptAuth.localLogIndex, Number(row.sourceLogIndex));
    assert.equal(row.sourceReceiptAuth.receipt.logs[Number(row.sourceLogIndex)].logIndex,
      `0x${(20 + Number(row.sourceLogIndex)).toString(16)}`);
  }
  assert.equal(primaryReceipts, 1);
  assert.equal(witnessReceipts, 1);
});

test('qualified outbound terminal binding survives advancing finality but rejects changed originals', async () => {
  for (const fault of ['none', 'receipt', 'effect', 'completion-id', 'height', 'time', 'tx-hash']) {
    const { receipt, transfer } = outbound();
    const ctx = context(receipt, transfer);
    await handleBatch(ctx);
    const completed = stored(ctx.store, transfer);
    if (fault === 'effect') completed.amount = '11';
    if (fault === 'completion-id') completed.completionAuth = { ...completed.completionAuth, completionId: 'other' };
    if (fault === 'height') completed.completedAtBlock = 12n;
    if (fault === 'time') completed.completedAt = new Date(0);
    if (fault === 'tx-hash') completed.completedAtTxHash = hash('cc');
    const retained = structuredClone(completed);
    const observation = [...ctx.store.rows.get(CompletedTransfer).values()][0];
    const savedObservation = structuredClone(observation);
    const fresh = structuredClone(receipt);
    if (fault === 'receipt') fresh.logs[0] = { ...fresh.logs[0],
      ...manager.abi.encodeEventLog(manager.abi.getEvent('Bridged'), [hash('44'), addr('55'), addr('66'), 11n]) };
    primaryRpc = evidenceRpc([fresh], 12);
    witnessRpc = primaryRpc;
    ctx.store.saves.length = 0;
    if (fault === 'none') {
      await handleBatch(ctx);
      assert.deepEqual(structuredClone(stored(ctx.store, completed)), retained);
      assert.equal(stored(ctx.store, completed).completionAuth.checkpointId, retained.completionAuth.checkpointId);
    } else {
      await assert.rejects(handleBatch(ctx), /HOLD:/, fault);
      assert.equal(ctx.saved.length, 0, fault);
    }
    assert.deepEqual(structuredClone(completed), retained, fault);
    assert.deepEqual(structuredClone(observation), savedObservation, fault);
  }
});

test('Gear settlement cannot authorize an invented or differently bound original deposit', async () => {
  for (const fault of ['witness-effect', 'agreed-effect', 'missing', 'transaction-index', 'local-log', 'tx-hash']) {
    const store = memoryStore(), state = new GearState();
    const transfer = new Transfer({ id: 'deposit', txHash: hash('bc'), sourceNetwork: Network.Ethereum,
      blockNumber: 100n, sourceTransactionIndex: 2n, sourceLogIndex: 3n, source: addr('66'),
      destination: hash('77'), sender: addr('55'), receiver: hash('44'), amount: '10', status: Status.Bridging });
    if (fault === 'transaction-index') transfer.sourceTransactionIndex = 4n;
    if (fault === 'local-log') transfer.sourceLogIndex = 23n;
    if (fault === 'tx-hash') transfer.txHash = hash('ff');
    await store.save(transfer);
    const receipt = inboundReceipt(), changed = structuredClone(receipt);
    changed.logs[3] = { ...changed.logs[3],
      ...manager.abi.encodeEventLog(manager.abi.getEvent('BridgingRequested'), [addr('55'), hash('44'), addr('66'), 11n]) };
    primaryRpc = evidenceRpc(fault === 'missing' ? [] : [fault === 'agreed-effect' ? changed : receipt], 101);
    witnessRpc = fault === 'witness-effect' ? evidenceRpc([changed], 101) : primaryRpc;
    await state.new({ store, log: quiet, blocks: [] });
    state.recordReceiptRelay({ slot: '200', block_number: 100, transaction_index: Number(transfer.sourceTransactionIndex) });
    state.recordReceiptSettlement({ slot: '200', transaction_index: String(transfer.sourceTransactionIndex),
      log_index: String(transfer.sourceLogIndex), deposit_count: '1', operation_id: hash('88'),
      eth_token_id: addr('66'), vara_token_id: hash('77'), sender: addr('55'), receiver: hash('44'),
      amount: '10', native: true }, new Date(block.timestamp), 300n, hash('dd'));
    if (fault === 'witness-effect') await assert.rejects(state.save(), /HOLD:/);
    else await state.save();
    assert.equal(stored(store, transfer).status, Status.Bridging, fault);
    assert.equal(stored(store, transfer).completedAt, undefined, fault);
    assert.ok((await store.find(ReceiptSettlement)).every((row) => !row.matched), fault);
  }
});

test('inbound terminal revalidation preserves bindings and never mutates originals on conflict', async () => {
  for (const outcome of [Status.Completed, Status.Refunded]) {
    const store = memoryStore(), state = new GearState();
    const transfer = new Transfer({ id: 'deposit', txHash: hash('bc'), sourceNetwork: Network.Ethereum,
      blockNumber: 100n, sourceTransactionIndex: 2n, sourceLogIndex: 3n, source: addr('66'),
      destination: hash('77'), sender: addr('55'), receiver: hash('44'), amount: '10', status: Status.Bridging });
    primaryRpc = evidenceRpc([inboundReceipt()], 101);
    witnessRpc = primaryRpc;
    await store.save(transfer);
    await state.new({ store, log: quiet, blocks: [] });
    state.recordReceiptRelay({ slot: '200', block_number: 100, transaction_index: 2 });
    const deposit = { slot: '200', transaction_index: '2', log_index: '3', deposit_count: '1',
      operation_id: hash('88'), eth_token_id: addr('66'), vara_token_id: hash('77'), sender: addr('55'),
      receiver: hash('44'), amount: '10', native: true };
    if (outcome === Status.Refunded) state.recordReceiptRefund({ ...deposit,
      child: hash('ab'), payout_child: hash('cd'), returned_value: '10' }, new Date(block.timestamp), 300n, hash('dd'));
    else state.recordReceiptSettlement(deposit, new Date(block.timestamp), 300n, hash('dd'));
    await state.save();
    const completed = stored(store, transfer), [settlement] = await store.find(ReceiptSettlement);
    const original = structuredClone(completed), originalSettlement = structuredClone(settlement);
    assert.equal(transfer.status, Status.Bridging);
    assert.equal(completed.status, outcome);
    primaryRpc = evidenceRpc([inboundReceipt()], 102);
    witnessRpc = primaryRpc;
    const clients = createEthereumEvidenceClients();
    const checkpoint = await qualifyFinality(clients);
    const authenticate = createReceiptAuthenticator(clients, checkpoint);
    const source = (row) => authenticateSourceReceipt(clients, row, addr('11'), checkpoint, authenticate);
    const revalidated = await reconcileReceiptSettlements(store, [settlement], source);
    assert.deepEqual(structuredClone(revalidated.transfers[0]), original);
    const malformedSource = structuredClone(completed.sourceReceiptAuth);
    delete malformedSource.receipt.cumulativeGasUsed;
    const malformed = new Transfer({ ...completed, sourceReceiptAuth: malformedSource });
    store.rows.get(Transfer).set(completed.id, malformed);
    await assert.rejects(reconcileReceiptSettlements(store, [settlement], source), /HOLD:/);
    assert.equal(malformed.sourceReceiptAuth.receipt.cumulativeGasUsed, undefined, 'qualification cannot repair retained qualified JSON');
    store.rows.get(Transfer).set(completed.id, completed);
    assert.deepEqual(structuredClone(completed), original);
    for (const change of [
      { id: 'changed-settlement' }, { blockNumber: 301n }, { txHash: hash('cc') }, { timestamp: new Date(0) },
      { deposit: { ...settlement.deposit, outcome: outcome === Status.Completed ? 'Refunded' : 'Completed' } },
      { deposit: { ...settlement.deposit, amount: '11' } },
    ]) {
      await assert.rejects(reconcileReceiptSettlements(store, [new ReceiptSettlement({ ...settlement, ...change })], source), /HOLD:/);
      assert.deepEqual(structuredClone(completed), original);
      assert.deepEqual(structuredClone(settlement), originalSettlement);
    }
    primaryRpc = evidenceRpc([], 102);
    witnessRpc = primaryRpc;
    const missing = createReceiptAuthenticator(clients, checkpoint);
    await assert.rejects(reconcileReceiptSettlements(store, [settlement],
      (row) => authenticateSourceReceipt(clients, row, addr('11'), checkpoint, missing)), /HOLD:/);
    assert.deepEqual(structuredClone(completed), original);
  }
});

test('multicall cannot reuse an earlier token effect or select ambiguous matching queue events', async () => {
  for (const duplicate of [false, true]) {
    const { receipt, transfer } = outbound();
    if (!duplicate) {
      receipt.logs[1] = event(queue.abi, 'MessageProcessed', [9n, hash('99'), 7n, addr('11')], addr('22'), 1);
    }
    receipt.logs.push(event(queue.abi, 'MessageProcessed', [9n, hash('77'), 8n, addr('11')], addr('22'), 2));
    const ctx = context(receipt, transfer);
    if (duplicate) await assert.rejects(handleBatch(ctx), /HOLD:/);
    else await handleBatch(ctx);
    assert.equal(stored(ctx.store, transfer).status, Status.Bridging);
    assert.equal(stored(ctx.store, transfer).completedAt, undefined);
    assert.ok(ctx.saved.filter((row) => row instanceof CompletedTransfer).every((row) => !row.matched));
  }
});


test('finalized original same-receipt effect is the only outbound completion', async () => {
  for (const mutate of [
    (r) => { r.logs.shift(); },
    (r) => { r.logs[0] = event(manager.abi, 'Bridged', [hash('44'), addr('55'), addr('66'), 11n], addr('11'), 0); },
    (r) => { r.logs[0].address = addr('99'); },
    (r) => { r.logs[0] = event(manager.abi, 'Bridged', [hash('99'), addr('55'), addr('66'), 10n], addr('11'), 0); },
    (r) => { r.logs[0] = event(manager.abi, 'Bridged', [hash('44'), addr('99'), addr('66'), 10n], addr('11'), 0); },
    (r) => { r.logs[0] = event(manager.abi, 'Bridged', [hash('44'), addr('55'), addr('99'), 10n], addr('11'), 0); },
    (r) => { r.logs[1] = event(queue.abi, 'MessageProcessed', [9n, hash('99'), 8n, addr('11')], addr('22'), 1); },
    (r) => { r.logs[1] = event(queue.abi, 'MessageProcessed', [8n, hash('77'), 8n, addr('11')], addr('22'), 1); },
  ]) {
    const { receipt, transfer } = outbound();
    mutate(receipt);
    const ctx = context(receipt, transfer);
    await handleBatch(ctx);
    assert.equal(stored(ctx.store, transfer).status, Status.Bridging);
    assert.equal(ctx.saved.filter((row) => row instanceof Transfer).length, 0);
    assert.ok(ctx.saved.filter((row) => row instanceof CompletedTransfer).every((row) => !row.matched));
  }
  const { receipt, transfer } = outbound();
  const ctx = context(receipt, transfer);
  await handleBatch(ctx);
  const completed = stored(ctx.store, transfer);
  assert.equal(transfer.status, Status.Bridging, 'qualification stages a clone, not the loaded original');
  assert.equal(completed.status, Status.Completed);
  assert.equal(completed.completedAtTxHash, txHash);
  const observation = [...ctx.store.rows.get(CompletedTransfer).values()][0];
  assert.equal(observation.matched, true);
  assert.equal(observation.receiptAuth.receipt.transactionHash, txHash);
  assert.equal(completed.completionAuth.completionId, observation.id);
  assert.equal(completed.completedAt.getTime(), block.timestamp);
  const later = outbound();
  later.receipt.logs[1] = event(queue.abi, 'MessageProcessed', [10n, hash('77'), 8n, addr('11')], addr('22'), 1);
  const laterCtx = context(later.receipt, later.transfer);
  await handleBatch(laterCtx);
  assert.equal(stored(laterCtx.store, later.transfer).status, Status.Completed);
});

test('outbound completion survives counterpart lag and restart while unrelated roots advance', async () => {
  const { receipt, transfer } = outbound();
  const store = memoryStore();
  store.findOneBy = async (entity, where) => (await store.find(entity, { where }))[0];
  const ctx = { ...context(receipt, undefined), store };
  const root = event(queue.abi, 'MerkleRoot', [12n, hash('88')], addr('22'), 2);
  receipt.logs.push(root);
  ctx.blocks[0].logs.push({ ...root, logIndex: 2, transactionIndex: 0 });
  await handleBatch(ctx);
  const retained = [...store.rows.get(CompletedTransfer).values()][0];
  assert.equal(retained.matched, false);
  assert.equal(retained.outboundReceipt.receipt.transactionHash, txHash);
  assert.equal(store.rows.get(Transfer), undefined);
  assert.equal([...store.rows.get(MerkleRootInMessageQueue).values()][0].blockNumber, 12n);
  // PostgreSQL JSONB may reorder keys when the durable original is replayed after restart.
  store.rows.get(CompletedTransfer).set(retained.id, new CompletedTransfer({ ...retained,
    outboundReceipt: Object.fromEntries(Object.entries(JSON.parse(JSON.stringify(retained.outboundReceipt))).reverse()) }));
  await handleBatch(ctx);
  assert.equal(store.rows.get(CompletedTransfer).size, 1);
  ctx.blocks = [];
  const originalMessageHash = transfer.ethBridgeBuiltInMsgHash;
  transfer.ethBridgeBuiltInMsgHash = null;
  await store.save(transfer);
  await handleBatch(ctx);
  assert.equal(transfer.status, Status.Bridging);
  transfer.ethBridgeBuiltInMsgHash = originalMessageHash;
  await handleBatch(ctx);
  assert.equal(stored(store, transfer).status, Status.Completed);
  assert.equal(stored(store, transfer).completedAtTxHash, txHash);
  assert.equal(stored(store, transfer).completedAtBlock, 11n);
  assert.equal(store.rows.get(CompletedTransfer).get(retained.id).matched, true);
  store.saves.length = 0;
  await handleBatch(ctx);
  assert.equal(store.saves.flat().filter((row) => row instanceof Transfer).length, 0);
});

test('pending outbound completion reauthenticates canonical history before matching a late transfer', async () => {
  const { receipt, transfer } = outbound();
  const store = memoryStore();
  store.findOneBy = async (entity, where) => (await store.find(entity, { where }))[0];
  await handleBatch({ ...context(receipt, undefined), store });
  await store.save(transfer);
  const changed = { ...context(receipt, transfer, hash('cc')), store, blocks: [] };
  await assert.rejects(handleBatch(changed), /HOLD:/);
  assert.equal(transfer.status, Status.Bridging);
  assert.equal([...store.rows.get(CompletedTransfer).values()][0].matched, false);
});

test('legacy completion rows and mismatched deferred effects never complete a late transfer', async () => {
  const { receipt, transfer } = outbound([hash('44'), addr('55'), addr('66'), 11n]);
  const store = memoryStore();
  store.findOneBy = async (entity, where) => (await store.find(entity, { where }))[0];
  await store.save(new CompletedTransfer({ id: 'legacy', srcNetwork: Network.Vara, destNetwork: Network.Ethereum,
    txHash, blockNumber: 11n, timestamp: new Date(block.timestamp), matched: false }));
  await handleBatch({ ...context(receipt, undefined), store });
  await store.save(transfer);
  await handleBatch({ ...context(receipt, transfer), store, blocks: [] });
  assert.equal(transfer.status, Status.Bridging);
  assert.ok([...store.rows.get(CompletedTransfer).values()].every((row) => !row.matched));
});

test('noncanonical batch fails before effects or cursor-owning handler return', async () => {
  const { receipt, transfer } = outbound();
  const ctx = context(receipt, transfer, hash('cc'));
  await assert.rejects(handleBatch(ctx), /HOLD:/);
  assert.equal(transfer.status, Status.Bridging);
  assert.equal(ctx.saved.length, 0);
});

function mockWait(t) {
  t.mock.timers.enable({ apis: ['setTimeout'] });
  syncBuiltinESMExports();
  t.after(() => { t.mock.timers.reset(); syncBuiltinESMExports(); });
}

function finalityContext(chain, heights, anchorHash) {
  const blockHash = (height) => hash(height.toString(16).padStart(2, '0'));
  let polls = 0, batches = 0;
  const rpc = {
    async call(method, params) {
      if (method === 'eth_getBlockByNumber' && params[0] === 'finalized') {
        const height = heights[Math.min(polls++, heights.length - 1)];
        return { number: '0x' + height.toString(16), hash: blockHash(height) };
      }
      if (method === 'chain_getFinalizedHead') return blockHash(heights[Math.min(polls++, heights.length - 1)]);
      if (method === 'chain_getHeader') return { number: '0x' + Number.parseInt(params[0].slice(2, 4), 16).toString(16) };
      const height = Number(BigInt(params[0]));
      const canonicalHash = anchorHash?.(height, polls) ?? blockHash(height);
      return chain === 'Ethereum' ? { number: params[0], hash: canonicalHash } : canonicalHash;
    },
    async batchCall(calls) {
      batches++;
      return calls.map(({ params }) => chain === 'Ethereum'
        ? { number: params[0], hash: blockHash(Number(BigInt(params[0]))) }
        : blockHash(params[0]));
    },
  };
  return { _chain: chain === 'Ethereum' ? { client: rpc } : { rpc },
    blocks: [{ header: { ...block, hash: blockHash(11) } }],
    get polls() { return polls; }, get batches() { return batches; } };
}

test('Ethereum lag HOLDs a batch without polling and completes on a fresh qualified attempt', async () => {
  const { receipt, transfer } = outbound();
  const ctx = context(receipt, transfer);
  const primary = evidenceRpc([receipt], 12), witness = evidenceRpc([receipt], 10);
  let polls = 0;
  primaryRpc = async (method, params) => {
    if (method === 'eth_getBlockByNumber' && params[0] === 'finalized') polls++;
    return primary(method, params);
  };
  witnessRpc = witness;
  await assert.rejects(handleBatch(ctx), /HOLD:/);
  assert.equal(polls, 1, 'Ethereum qualification is bounded, not a finality wait loop');
  assert.equal(stored(ctx.store, transfer).status, Status.Bridging);
  assert.equal(ctx.saved.length, 0);
  witnessRpc = evidenceRpc([receipt], 11);
  await handleBatch(ctx);
  assert.equal(stored(ctx.store, transfer).status, Status.Completed);
  assert.equal([...ctx.store.rows.get(EthereumFinalityCheckpoint).values()][0].blockNumber, 11n);
});

test('Ethereum reload rejects saved-head regression and a changed prior anchor', async () => {
  for (const fault of ['regression', 'anchor']) {
    const { receipt, transfer } = outbound();
    const ctx = context(receipt, transfer);
    await handleBatch(ctx);
    const original = structuredClone(stored(ctx.store, transfer));
    ctx.blocks = [];
    const canonical = evidenceRpc([receipt], fault === 'regression' ? 10 : 12);
    primaryRpc = async (method, params) => {
      const result = await canonical(method, params);
      if (fault === 'anchor' && method === 'eth_getBlockByNumber' && params[0] === '0xb') {
        return { ...result, hash: hash('cc') };
      }
      return result;
    };
    witnessRpc = primaryRpc;
    ctx.store.saves.length = 0;
    await assert.rejects(handleBatch(ctx), /HOLD:/);
    assert.deepEqual(structuredClone(stored(ctx.store, transfer)), original);
    assert.equal(ctx.store.saves.length, 0);
  }
});

test('Ethereum fails closed on malformed, unavailable and noncanonical independent evidence', async () => {
  for (const fault of ['finality', 'transport', 'missing-header', 'canonical', 'ancestry']) {
    const { receipt, transfer } = outbound();
    const ctx = context(receipt, transfer);
    const canonical = evidenceRpc([receipt], fault === 'ancestry' ? 12 : 11);
    witnessRpc = async (method, params) => {
      if (fault === 'transport') throw new Error('transport unavailable');
      const result = await canonical(method, params);
      if (method !== 'eth_getBlockByNumber') return result;
      if (fault === 'finality' && params[0] === 'finalized') return null;
      if (fault === 'missing-header' && params[0] === '0xb') return null;
      if (fault === 'canonical' && params[0] === '0xb') return { ...result, hash: hash('cc') };
      if (fault === 'ancestry' && params[0] === 'finalized') return { ...result, parentHash: hash('cc') };
      return result;
    };
    await assert.rejects(requireEthFinality(ctx), /HOLD:/, fault);
    assert.equal(transfer.status, Status.Bridging);
    assert.equal(ctx.saved.length, 0);
  }
});

test('Gear finality lag holds the entire batch beyond 120 seconds before any state access', async (t) => {
  mockWait(t);
  const ctx = finalityContext('Gear', [10, 10, 10, 10, 10, 11]);
  const store = memoryStore(), state = new GearState();
  Object.assign(ctx, { store, log: quiet });
  let finished = false;
  const pending = requireGearFinality(ctx).then(async () => {
    t.mock.timers.reset();
    syncBuiltinESMExports();
    await state.new(ctx);
    await state.save();
    finished = true;
  });
  for (let i = 0; i < 4; i++) {
    await new Promise(setImmediate);
    t.mock.timers.tick(30_000);
  }
  await new Promise(setImmediate);
  assert.equal(ctx.polls, 5);
  assert.equal(ctx.batches, 0);
  assert.equal(finished, false);
  assert.equal(store.queries.length, 0);
  assert.equal(store.saves.length, 0);
  t.mock.timers.tick(29_999);
  await new Promise(setImmediate);
  assert.equal(finished, false);
  t.mock.timers.tick(1);
  await pending;
  assert.equal(ctx.polls, 6);
  assert.equal(ctx.batches, 1);
  assert.equal(finished, true);
});

for (const [chain, requireFinality] of [['Gear', requireGearFinality]]) {
  test(chain + ' rejects regression from the immediately previous finalized height', async (t) => {
    mockWait(t);
    const ctx = finalityContext(chain, [20, 21, 20]);
    ctx.blocks[0].header.height = 22;
    const rejection = assert.rejects(requireFinality(ctx), /height regressed/);
    for (let i = 0; i < 2; i++) {
      await new Promise(setImmediate);
      t.mock.timers.tick(30_000);
    }
    await rejection;
    assert.equal(ctx.batches, 0);
  });

  test(chain + ' revalidates each previous anchor, not just the initial one', async (t) => {
    mockWait(t);
    const ctx = finalityContext(chain, [20, 21, 22], (height, polls) => height === 21 && polls === 3 ? hash('cc') : undefined);
    ctx.blocks[0].header.height = 22;
    const rejection = assert.rejects(requireFinality(ctx), /previous finalized .* anchor changed/);
    for (let i = 0; i < 2; i++) {
      await new Promise(setImmediate);
      t.mock.timers.tick(30_000);
    }
    await rejection;
    assert.equal(ctx.batches, 0);
  });

  test(chain + ' fails closed on malformed finality, transport failure and incomplete or noncanonical batches', async () => {
    for (const failure of ['finality', 'transport', 'incomplete', 'canonical', 'anchor']) {
      const ctx = finalityContext(chain, [11], failure === 'anchor' ? () => hash('cc') : undefined);
      const rpc = ctx._chain.client ?? ctx._chain.rpc;
      if (failure === 'finality') rpc.call = async () => null;
      if (failure === 'transport') rpc.call = async () => { throw new Error('transport unavailable'); };
      if (failure === 'incomplete') rpc.batchCall = async () => [];
      if (failure === 'canonical') rpc.batchCall = async () => chain === 'Ethereum'
        ? [{ number: '0xb', hash: hash('cc') }] : [hash('cc')];
      await assert.rejects(requireFinality(ctx), failure === 'transport' ? /transport unavailable/ : /HOLD:/);
    }
  });
}


test('Gear proof envelope and queued native outcome remain pending until exact settled log', async () => {
  const store = memoryStore();
  const transfer = new Transfer({ id: 'deposit', txHash: hash('bc'), sourceNetwork: Network.Ethereum, blockNumber: 100n,
    sourceTransactionIndex: 2n, sourceLogIndex: 3n, source: addr('66'), destination: hash('77'),
    sender: addr('55'), receiver: hash('44'), amount: '10', status: Status.Bridging });
  await store.save(transfer);
  const state = new GearState();
  const ctx = { store, log: quiet, blocks: [] };
  await state.new(ctx);
  state.recordReceiptRelay({ slot: '200', block_number: 100, transaction_index: 2 });
  await state.save();
  assert.equal(transfer.status, Status.Bridging);
  const deposit = { slot: '200', transaction_index: '2', log_index: '3', deposit_count: '1',
    operation_id: hash('88'), eth_token_id: addr('66'), vara_token_id: hash('77'), sender: addr('55'),
    receiver: hash('44'), amount: '10', native: true };
  await state.new(ctx);
  state.recordReceiptSettlement({ ...deposit, amount: '11' }, new Date(), 300n, hash('bb'));
  await state.save();
  assert.equal(transfer.status, Status.Bridging);
  assert.equal((await store.find(ReceiptSettlement))[0].matched, false);
  // A conflicting second observation cannot replace the original authenticated outcome.
  await state.new(ctx);
  state.recordReceiptSettlement(deposit, new Date(), 301n, hash('cc'));
  await assert.rejects(state.save(), /conflicting receipt deposit/);
  // A distinct correctly bound original log settles, while the mismatched row stays pending.
  const another = new Transfer({ ...transfer, id: 'another', sourceLogIndex: 4n });
  await store.save(another);
  await state.new(ctx);
  state.recordReceiptSettlement({ ...deposit, log_index: '4' }, new Date(), 302n, hash('dd'));
  await state.save();
  assert.equal(stored(store, another).status, Status.Completed);
  assert.equal(transfer.status, Status.Bridging);
  assert.equal((await store.find(ReceiptRelay)).length, 1);
});

test('decoded original native refunds remain distinct from delivery and bind the full receipt effect', async () => {
  const { readFileSync } = await import('node:fs');
  const { Sails } = await import('sails-js');
  const { SailsIdlParser } = await import('sails-js-parser');
  const { Decoder } = await import('../lib/gear/codec.js');
  const { handleVftManagerEvents } = await import('../lib/gear/handlers/vft-manager.js');
  const parser = new SailsIdlParser();
  await parser.init();
  const sails = new Sails(parser).parseIdl(readFileSync(new URL('../../../api/gear/vft_manager.idl', import.meta.url), 'utf8'));
  const decoder = new Decoder(sails);
  const deposit = { slot: '200', transaction_index: '2', log_index: '3', deposit_count: '1',
    operation_id: hash('88'), eth_token_id: addr('66'), vara_token_id: hash('77'), sender: addr('55'),
    receiver: hash('44'), amount: '10', native: true, child: hash('aa'), payout_child: hash('bb'), returned_value: '10' };
  const dispatch = async (state, value, method = 'ReceiptDepositRefunded') => {
    const eventType = sails.services.VftManager.events[method].type;
    const payload = sails.registry.createType(`(String,String,${JSON.stringify(eventType)})`, ['VftManager', method, value]).toHex();
    await handleVftManagerEvents({ state, decoder, service: decoder.service(payload), method: decoder.method(payload),
      blockHeader: { height: 300, timestamp: block.timestamp }, log: quiet,
      event: { args: { message: { id: hash('cc'), source: process.env.GEAR_VFT_MANAGER, destination: hash('00'), payload } },
        extrinsic: { hash: hash('dd') } } });
  };
  for (const mismatch of ['none', 'source', 'wrapper', 'sender', 'recipient', 'amount', 'block', 'transaction', 'log',
    'partial', 'over-return', 'native', 'operation', 'child', 'payout-child', 'same-child']) {
    const store = memoryStore(), state = new GearState(), ctx = { store, log: quiet, blocks: [] };
    const transfer = new Transfer({ id: 'original', txHash: hash('bc'), sourceNetwork: Network.Ethereum, blockNumber: 100n,
      sourceTransactionIndex: 2n, sourceLogIndex: 3n, source: addr('66'), destination: hash('77'),
      sender: addr('55'), receiver: hash('44'), amount: '10', status: Status.Bridging });
    await store.save(transfer);
    await state.new(ctx);
    state.recordReceiptRelay({ slot: '200', block_number: 100, transaction_index: 2 });
    await state.save();
    const value = { ...deposit };
    if (mismatch === 'source') value.eth_token_id = addr('99');
    if (mismatch === 'wrapper') value.vara_token_id = hash('99');
    if (mismatch === 'sender') value.sender = addr('99');
    if (mismatch === 'recipient') value.receiver = hash('99');
    if (mismatch === 'amount') value.amount = value.returned_value = '11';
    if (mismatch === 'block') transfer.blockNumber = 101n;
    if (mismatch === 'transaction') value.transaction_index = '4';
    if (mismatch === 'log') value.log_index = '4';
    if (mismatch === 'partial') value.returned_value = '9';
    if (mismatch === 'over-return') value.returned_value = '11';
    if (mismatch === 'native') value.native = false;
    if (mismatch === 'operation') value.operation_id = hash('00');
    if (mismatch === 'child') value.child = hash('00');
    if (mismatch === 'payout-child') value.payout_child = hash('00');
    if (mismatch === 'same-child') value.payout_child = value.child;
    await state.new(ctx);
    if (['partial', 'over-return', 'native', 'operation', 'child', 'payout-child', 'same-child'].includes(mismatch)) {
      await assert.rejects(dispatch(state, value), /HOLD: native refund/);
      assert.equal(transfer.status, Status.Bridging, mismatch);
      assert.equal((await store.find(ReceiptSettlement)).length, 0, mismatch);
      continue;
    }
    await dispatch(state, value);
    await state.save();
    const [settlement] = await store.find(ReceiptSettlement);
    if (mismatch !== 'none') {
      assert.equal(transfer.status, Status.Bridging, mismatch);
      assert.equal(settlement.matched, false, mismatch);
      continue;
    }
    const refunded = stored(store, transfer);
    assert.equal(refunded.status, Status.Refunded);
    assert.notEqual(refunded.status, Status.Completed);
    assert.equal(refunded.receiptSlot, 200n);
    assert.equal(refunded.completedAtBlock, 300n);
    assert.equal(refunded.completedAtTxHash, hash('dd'));
    assert.equal(settlement.matched, true);
    assert.equal(settlement.deposit.outcome, 'Refunded');
    for (const field of ['operation_id', 'vara_token_id', 'child', 'payout_child', 'receiver']) {
      assert.equal(settlement.deposit[field], deposit[field], field);
    }
    assert.equal(BigInt(settlement.deposit.returned_value), 10n);
    // An authenticated terminal record cannot become delivery or another payout operation.
    for (const change of [{ operation_id: hash('99') }, { child: hash('99') }, { payout_child: hash('99') }]) {
      await state.new(ctx);
      await dispatch(state, { ...deposit, ...change });
      await assert.rejects(state.save(), /conflicting receipt deposit/);
      assert.equal(stored(store, transfer).status, Status.Refunded);
    }
    await state.new(ctx);
    await dispatch(state, deposit, 'ReceiptDepositSettled');
    await assert.rejects(state.save(), /conflicting receipt deposit/);
    assert.equal(stored(store, transfer).status, Status.Refunded);
  }
});

test('bounded pending pages cycle and match authenticated tuples arriving in later batches', async () => {
  const store = memoryStore();
  const deposit = { slot: '200', transaction_index: '2', log_index: '3', deposit_count: '1',
    operation_id: hash('88'), eth_token_id: addr('66'), vara_token_id: hash('77'), sender: addr('55'),
    receiver: hash('44'), amount: '10', native: true };
  const pending = Array.from({ length: 1001 }, (_, index) => new ReceiptSettlement({
    id: String(index).padStart(4, '0'), deposit: { ...deposit, slot: String(200 + index) },
    timestamp: new Date(1_700_000_000_000), blockNumber: 300n, txHash: hash('bb'), matched: false,
  }));
  await store.save(pending);
  const state = new GearState(), ctx = { store, log: quiet, blocks: [{ header: { ...block, height: 300 } }] };
  await state.new(ctx);
  await state.save();
  assert.equal(store.queries.find(({ entity, options }) => entity === ReceiptSettlement && options.where?.matched === false).options.take, 1000);
  // Replaying the same batch models a transaction retry: it must not skip the first page.
  const beforeRetry = store.queries.length;
  await state.new(ctx);
  await state.save();
  const retryPage = store.queries.slice(beforeRetry).find(({ entity, options }) =>
    entity === ReceiptSettlement && options.where?.matched === false);
  assert.equal(retryPage.options.where.id, undefined);
  const transfer = new Transfer({ id: 'late', txHash: hash('bc'), sourceNetwork: Network.Ethereum, blockNumber: 100n,
    sourceTransactionIndex: 2n, sourceLogIndex: 3n, source: addr('66'), destination: hash('77'),
    sender: addr('55'), receiver: hash('44'), amount: '10', status: Status.Bridging });
  await store.save([transfer]);
  ctx.blocks[0].header = { ...block, height: 301, hash: hash('ab') };
  await state.new(ctx);
  state.recordReceiptRelay({ slot: '1200', block_number: 100, transaction_index: 2 });
  const before = store.queries.length;
  await state.save();
  assert.equal(stored(store, transfer).status, Status.Completed);
  assert.equal(stored(store, pending[1000]).matched, true);
  assert.equal(stored(store, pending[0]).matched, false);
  assert.ok(store.queries.slice(before).filter(({ entity }) => entity !== EthereumFinalityCheckpoint).length <= 6);
  const first = new Transfer({ ...transfer, id: 'first', txHash: hash('bd'), blockNumber: 101n, receiptSlot: null, status: Status.Bridging });
  await store.save([first]);
  ctx.blocks[0].header = { ...block, height: 302, hash: hash('ac') };
  await state.new(ctx);
  state.recordReceiptRelay({ slot: '200', block_number: 101, transaction_index: 2 });
  await state.save();
  assert.equal(stored(store, first).status, Status.Completed);
  assert.equal(stored(store, pending[0]).matched, true);
  assert.ok(store.saves.every(Array.isArray));
  assert.ok(store.queries.filter(({ entity, options }) => entity === ReceiptSettlement && options.where?.matched === false)
    .every(({ options }) => options.take === 1000 && options.order.id === 'ASC'));
});


test('batched receipt joins retain every authenticated tuple, effect and slot check', async () => {
  const deposit = { slot: '200', transaction_index: '2', log_index: '3', deposit_count: '1',
    operation_id: hash('88'), eth_token_id: addr('66'), vara_token_id: hash('77'), sender: addr('55'),
    receiver: hash('44'), amount: '10', native: true };
  for (const field of ['sourceNetwork', 'blockNumber', 'sourceTransactionIndex', 'sourceLogIndex',
    'source', 'destination', 'sender', 'receiver', 'amount', 'receiptSlot', 'duplicate']) {
    const store = memoryStore();
    const transfer = new Transfer({ id: 'original', txHash: hash('bc'), sourceNetwork: Network.Ethereum, blockNumber: 100n,
      sourceTransactionIndex: 2n, sourceLogIndex: 3n, source: addr('66'), destination: hash('77'),
      sender: addr('55'), receiver: hash('44'), amount: '10', status: Status.Bridging });
    if (field === 'sourceNetwork') transfer[field] = Network.Vara;
    else if (['blockNumber', 'sourceTransactionIndex', 'sourceLogIndex', 'receiptSlot'].includes(field)) transfer[field] = 999n;
    else if (field !== 'duplicate') transfer[field] = field === 'amount' ? '11' : hash('99');
    await store.save([transfer, ...(field === 'duplicate' ? [new Transfer({ ...transfer, id: 'duplicate' })] : [])]);
    const state = new GearState();
    await state.new({ store, log: quiet, blocks: [] });
    state.recordReceiptRelay({ slot: '200', block_number: 100, transaction_index: 2 });
    state.recordReceiptSettlement(deposit, new Date(), 300n, hash('bb'));
    if (field === 'receiptSlot' || field === 'duplicate') await assert.rejects(state.save(), /HOLD:/);
    else {
      await state.save();
      assert.equal((await store.find(ReceiptSettlement))[0].matched, false, field);
    }
    assert.equal(transfer.status, Status.Bridging, field);
  }
  const store = memoryStore(), state = new GearState(), ctx = { store, log: quiet, blocks: [] };
  await state.new(ctx);
  state.recordReceiptRelay({ slot: '200', block_number: 100, transaction_index: 2 });
  await state.save();
  await state.new(ctx);
  state.recordReceiptRelay({ slot: '200', block_number: 101, transaction_index: 2 });
  await assert.rejects(state.save(), /conflicting authenticated receipt block/);
});

test('Ethereum RPC ingestion uses canonical receipts, not unrestricted log scans', async () => {
  const { processor } = await import('../lib/eth/processor.js');
  const { toMappingRequest } = await import('@subsquid/evm-processor/lib/ds-rpc/request.js');
  for (const { request } of processor.getBatchRequests()) {
    assert.equal(request.logs.length, 3);
    assert.ok(request.logs.every((log) => log.transaction && log.address.length === 1));
    const mapping = toMappingRequest(request);
    assert.equal(mapping.receipts, true);
    assert.equal(mapping.logs, false);
  }
});
test('RPC-only ingestion is explicit and preserves default gateway credentials', () => {
  for (const chain of ['eth', 'gear']) {
    const prefix = chain.toUpperCase();
    const script = `const { config } = await import('./lib/${chain}/config.js');
      const module = await import('./lib/${chain}/processor.js');
      const processor = module.processor ?? module.getProcessor();
      console.log(JSON.stringify({ rpcOnly: config.rpcOnly, archive: processor.archive }));`;
    const invoke = (mode, apiKey) => {
      const env = { ...process.env };
      delete env[`${prefix}_RPC_ONLY`];
      delete env[`${prefix}_SQUID_API_KEY`];
      if (mode !== undefined) env[`${prefix}_RPC_ONLY`] = mode;
      if (apiKey !== undefined) env[`${prefix}_SQUID_API_KEY`] = apiKey;
      return spawnSync(process.execPath, ['--input-type=module', '-e', script], {
        cwd: fileURLToPath(new URL('..', import.meta.url)), env, encoding: 'utf8',
      });
    };
    for (const mode of [undefined, 'false', 'TRUE']) {
      const missing = invoke(mode);
      assert.notEqual(missing.status, 0);
      assert.match(missing.stderr, new RegExp(`Missing env: ${prefix}_SQUID_API_KEY`));
      const normal = invoke(mode, 'local-test');
      assert.equal(normal.status, 0, normal.stderr);
      const result = JSON.parse(normal.stdout.trim().split('\n').at(-1));
      assert.equal(result.rpcOnly, false);
      assert.equal(result.archive.apiKey, 'local-test');
      assert.match(result.archive.url, /archive\.subsquid\.io/);
    }
    const local = invoke('true');
    assert.equal(local.status, 0, local.stderr);
    assert.deepEqual(JSON.parse(local.stdout.trim().split('\n').at(-1)), { rpcOnly: true });
  }
});

test('authenticated Gear program rotations survive sequential dispatch, reload and restart with stale configuration', async (t) => {
  const { default: pg } = await import('pg');
  const ids = await import('../lib/gear/programIds.js');
  const { getProcessor } = await import('../lib/gear/processor.js');
  const { readFileSync } = await import('node:fs');
  const { Sails } = await import('sails-js');
  const { SailsIdlParser } = await import('sails-js-parser');
  // Each client shares durable rows, not canned SELECT responses or the initializer's input.
  const rows = new Map();
  let tableExists = false;
  t.mock.method(pg.Client.prototype, 'connect', async () => {});
  t.mock.method(pg.Client.prototype, 'end', async () => {});
  t.mock.method(pg.Client.prototype, 'query', async (sql, params) => {
    const query = sql.replace(/\s+/g, ' ').trim();
    if (query.startsWith('SELECT EXISTS')) return { rows: [{ exists: tableExists }] };
    if (query.startsWith('CREATE TABLE gear_programs')) {
      assert.equal(tableExists, false);
      tableExists = true;
      return { rows: [] };
    }
    assert.equal(tableExists, true);
    if (query.startsWith('INSERT INTO gear_programs')) {
      const [name, program_id] = params;
      if (!rows.has(name) || query.includes('DO UPDATE')) rows.set(name, program_id);
      return { rows: [] };
    }
    if (query.startsWith('UPDATE gear_programs')) {
      const [program_id, name] = params;
      if (rows.has(name)) rows.set(name, program_id);
      return { rows: [] };
    }
    if (query.startsWith('SELECT name, program_id FROM gear_programs')) {
      return { rows: [...rows].filter(([name]) => params[0].includes(name))
        .map(([name, program_id]) => ({ name, program_id })) };
    }
    throw new Error(`Unexpected program ID query: ${query}`);
  });
  let receiveHandler;
  const ready = new Promise((resolve) => { receiveHandler = resolve; });
  t.mock.method(getProcessor(), 'run', (_db, handler) => { receiveHandler(handler); });
  await import('../lib/gear/main.js');
  const handleGearBatch = await ready;
  assert.deepEqual([...rows.values()].sort(), [hash('11'), hash('22'), hash('33')].sort());

  const parser = new SailsIdlParser();
  await parser.init();
  const sails = new Sails(parser).parseIdl(readFileSync(new URL('../../../api/gear/vft_manager.idl', import.meta.url), 'utf8'));
  const method = 'HistoricalProxyAddressChanged';
  const eventType = sails.services.VftManager.events[method].type;
  const managerId = hash('55'), proxyId = hash('66');
  const proxyEvent = (source, next) => ({
    name: 'Gear.UserMessageSent', extrinsic: { hash: hash('dd') },
    args: { message: { id: hash('cc'), source, destination: hash('00'),
      payload: sails.registry.createType(`(String,String,${JSON.stringify(eventType)})`,
        ['VftManager', method, { old: hash('22'), new: next }]).toHex() } },
  });
  const ctx = finalityContext('Gear', [11]);
  Object.assign(ctx, { store: memoryStore(), log: quiet, isHead: false });
  ctx.blocks[0].header._runtime = { decodeStorageValue(name, data) {
    assert.equal(name, 'GearProgram.ProgramStorage');
    assert.equal(data, 'exited-manager');
    return { __kind: 'Exited', value: managerId };
  } };
  const call = ctx._chain.rpc.call;
  ctx._chain.rpc.call = async (method, params) => {
    if (method !== 'state_getStorage') return call(method, params);
    assert.ok(params[0].endsWith(hash('11').slice(2)));
    assert.equal(params[1], ctx.blocks[0].header.hash);
    return 'exited-manager';
  };
  ctx.blocks[0].events = [
    { name: 'Gear.ProgramChanged', args: { id: hash('11'), change: { __kind: 'Inactive' } } },
    proxyEvent(managerId, proxyId),
    proxyEvent(hash('11'), hash('99')), // The old source must no longer be trusted.
  ];
  await handleGearBatch(ctx);
  assert.equal(rows.get('vft_manager'), managerId);
  assert.equal(rows.get('historical_proxy'), proxyId);
  const expected = new Map([[managerId, 'vft_manager'], [proxyId, 'historical_proxy'], [hash('33'), 'bridging_payment']]);
  assert.deepEqual(ids.programs, expected);
  await ids.setPrograms();
  assert.deepEqual(ids.programs, expected);

  // A new module has no in-memory registry; existing rows win and missing roles are seeded.
  rows.delete('bridging_payment');
  rows.set('retired_role', hash('88'));
  const restarted = await import('../lib/gear/programIds.js?rotation-restart');
  assert.equal(restarted.programs, undefined);
  await restarted.setPrograms();
  assert.deepEqual(restarted.programs, expected);
  assert.equal(rows.get('bridging_payment'), hash('33'));
  assert.equal(rows.get('retired_role'), hash('88'));

  const { config: gearConfig } = await import('../lib/gear/config.js');
  const { GearManagerHandoff, Pair } = await import('../lib/model/index.js');
  gearConfig.retainedCutoverFromBlock = 12;
  t.after(() => { gearConfig.retainedCutoverFromBlock = undefined; });
  const successor = hash('77'), third = hash('88'), fingerprint = hash('ab');
  const active = { phase: 'Active', predecessor: null, successor: null, minimum_receipt_slot: '0',
    last_receipt_slot: null, config_fingerprint: null, transfer_count: 0, completed_transfer_count: 0,
    in_flight_transfer_count: 0 };
  const retired = { ...active, phase: 'Retired', successor, last_receipt_slot: '200',
    config_fingerprint: fingerprint, transfer_count: 1, completed_transfer_count: 1 };
  const prepared = { ...active, phase: 'AwaitingSource', predecessor: managerId, minimum_receipt_slot: '201',
    config_fingerprint: fingerprint };
  const managerEvent = (source, method, value, id = hash('cc')) => ({
    name: 'Gear.UserMessageSent', extrinsic: { hash: hash('dd') },
    args: { message: { id, source, destination: hash('00'),
      payload: sails.registry.createType(`(String,String,${JSON.stringify(sails.services.VftManager.events[method].type)})`,
        ['VftManager', method, value]).toHex() } },
  });
  const requests = [];
  const batch = (height, store, statuses, events = []) => {
    const result = finalityContext('Gear', [height]);
    Object.assign(result, { store, log: quiet, isHead: false });
    Object.assign(result.blocks[0], { events, header: { ...block, height, hash: hash(height.toString(16).padStart(2, '0')) } });
    const original = result._chain.rpc.call;
    result._chain.rpc.call = async (method, params) => {
      if (method !== 'gear_calculateReplyForHandle') return original(method, params);
      assert.equal(params[5], result.blocks[0].header.hash, 'queries must authenticate the exact finalized batch block');
      requests.push(params[1]);
      const status = statuses[params[1]];
      if (status instanceof Error) throw status;
      assert.ok(status, `Unexpected query to ${params[1]}`);
      return { code: { Success: {} }, payload: sails.registry.createType(
        `(String,String,${sails.services.VftManager.queries.CutoverStatus.returnType})`,
        ['VftManager', 'CutoverStatus', status]).toHex() };
    };
    return result;
  };

  // No capability query before the explicit boundary, including immutable old programs.
  await handleGearBatch(batch(11, memoryStore(), { [managerId]: new Error('unsupported legacy query') }));
  for (const [name, badSource, badDestination] of [
    ['predecessor', retired, { ...prepared, phase: 'Active', predecessor: hash('99') }],
    ['floor', retired, { ...prepared, phase: 'Active', minimum_receipt_slot: '200' }],
    ['fingerprint', retired, { ...prepared, phase: 'Active', config_fingerprint: hash('99') }],
    ['custody', { ...retired, completed_transfer_count: 0 }, { ...prepared, phase: 'Active' }],
    ['unresolved', { ...retired, in_flight_transfer_count: 1 }, { ...prepared, phase: 'Active' }],
    ['cycle', { ...retired, successor: managerId }, prepared],
  ]) {
    const store = memoryStore();
    await assert.rejects(handleGearBatch(batch(12, store, { [managerId]: badSource, [successor]: badDestination })), /HOLD:/, name);
    assert.equal((await store.find(GearManagerHandoff)).length, 0, name);
    assert.equal((await store.find(Transfer)).length, 0, name);
  }
  await assert.rejects(handleGearBatch(batch(12, memoryStore(), { [managerId]: new Error('archive RPC unavailable') })),
    /archive RPC unavailable/);

  const store = memoryStore();
  await store.save([new Pair({ id: 'pair', varaToken: hash('77'), ethToken: addr('66'), isRemoved: false }),
    ...['predecessor-deposit', 'successor-deposit', 'successor-refund'].map((id, index) => new Transfer({
      id, txHash: hash('bc'), sourceNetwork: Network.Ethereum, destNetwork: Network.Vara, blockNumber: 100n,
      sourceTransactionIndex: 2n, sourceLogIndex: BigInt(index + 3), source: addr('66'), destination: hash('77'),
      sender: addr('55'), receiver: hash('44'), amount: '10', status: Status.Bridging,
    })),
    new ReceiptRelay({ id: '200:2', blockNumber: 100n }), new ReceiptRelay({ id: '201:2', blockNumber: 100n })]);
  const deposit = { slot: '200', transaction_index: '2', log_index: '3', deposit_count: '1',
    operation_id: hash('98'), eth_token_id: addr('66'), vara_token_id: hash('77'), sender: addr('55'),
    receiver: hash('44'), amount: '10', native: true };
  const outgoing = (nonce) => ({ nonce, queue_id: nonce, hash: hash('ac'), vara_token_id: hash('77'),
    amount: '10', sender: hash('44'), receiver: addr('55') });
  // Source stays runtime-Active/contract-Retired, with no ProgramChanged/Inactive event.
  // The normal AwaitingSource interval neither admits successor effects nor stalls history.
  await handleGearBatch(batch(12, store, { [managerId]: retired, [successor]: prepared }, [
    managerEvent(managerId, 'ReceiptDepositSettled', deposit),
    managerEvent(managerId, 'BridgingRequested', outgoing('1'), hash('c1')),
    managerEvent(successor, 'BridgingRequested', outgoing('2'), hash('c2')),
  ]));
  assert.equal((await store.find(GearManagerHandoff)).length, 0);
  assert.equal((await store.find(Transfer)).find(({ id }) => id === 'predecessor-deposit').status, Status.Completed);
  assert.equal((await store.find(Transfer)).some(({ id }) => id === hash('c2')), false);

  const refund = { ...deposit, slot: '201', log_index: '5', operation_id: hash('99'),
    child: hash('aa'), payout_child: hash('bb'), returned_value: '10' };
  const live = batch(13, store, { [managerId]: retired, [successor]: { ...prepared, phase: 'Active' } }, [
    managerEvent(successor, 'BridgingRequested', outgoing('2'), hash('c2')),
    managerEvent(successor, 'ReceiptDepositSettled', { ...deposit, slot: '201', log_index: '4' }),
    managerEvent(successor, 'ReceiptDepositRefunded', refund),
  ]);
  await handleGearBatch(live);
  assert.equal(rows.get('vft_manager'), managerId, 'the durable original role remains the authenticated trust anchor');
  const [handoff] = await store.find(GearManagerHandoff);
  assert.equal(handoff.id, managerId);
  assert.equal(handoff.successor, successor);
  assert.equal(handoff.minimumReceiptSlot, '0');
  assert.equal(handoff.lastReceiptSlot, '200');
  assert.equal(handoff.blockNumber, 13n);
  const projected = await store.find(Transfer);
  assert.equal(projected.find(({ id }) => id === 'successor-deposit').status, Status.Completed);
  assert.equal(projected.find(({ id }) => id === 'successor-refund').status, Status.Refunded);
  assert.ok(projected.some(({ id }) => id === hash('c1')));
  assert.ok(projected.some(({ id }) => id === hash('c2')));

  // Rehydration uses committed links, not stale config or an in-memory discovery cache.
  await ids.setPrograms();
  const restartReady = new Promise((resolve) => {
    t.mock.method(getProcessor(), 'run', (_db, handler) => resolve(handler));
  });
  await import('../lib/gear/main.js?retained-restart');
  const handleRestartedBatch = await restartReady;
  requests.length = 0;
  await handleRestartedBatch(batch(14, store, { [successor]: { ...prepared, phase: 'Draining', successor: third,
    last_receipt_slot: '300' } }));
  assert.deepEqual(requests, [successor], 'do not requery every retained ancestor on every block');
  assert.equal((await store.find(GearManagerHandoff)).length, 1);
  for (const status of [prepared, { ...prepared, phase: 'Active', predecessor: hash('99') }]) {
    await assert.rejects(handleRestartedBatch(batch(14, store, { [successor]: status })),
      /readiness or predecessor changed/);
  }
  for (const [source, value] of [[managerId, { ...deposit, slot: '201' }],
    [successor, { ...deposit, slot: '200' }]]) {
    await assert.rejects(handleGearBatch(batch(14, store, { [successor]: { ...prepared, phase: 'Active' } },
      [managerEvent(source, 'ReceiptDepositSettled', value)])), /outside authenticated manager deployment domain/);
  }
  // Historical replay ignores a persisted future link and reauthenticates at its original block.
  await handleGearBatch(batch(12, store, { [managerId]: retired, [successor]: prepared },
    [managerEvent(managerId, 'BridgingRequested', outgoing('1'), hash('c1'))]));
  assert.equal((await store.find(GearManagerHandoff)).length, 1);
  const secondRetired = { ...prepared, phase: 'Retired', successor: third, last_receipt_slot: '300',
    transfer_count: 1, completed_transfer_count: 1 };
  await handleGearBatch(batch(15, store, { [successor]: secondRetired,
    [third]: { ...active, predecessor: successor, minimum_receipt_slot: '301', config_fingerprint: fingerprint } }));
  assert.equal((await store.find(GearManagerHandoff)).length, 2);
  for (const changed of [undefined, 13]) {
    gearConfig.retainedCutoverFromBlock = changed;
    await assert.rejects(handleGearBatch(batch(16, store, {})), /capability boundary changed or removed/);
  }
  gearConfig.retainedCutoverFromBlock = 12;
  for (const invalid of ['-1', '1.5', 'no', '', '9007199254740992']) {
    const result = spawnSync(process.execPath, ['--input-type=module', '-e', "await import('./lib/gear/config.js')"], {
      cwd: fileURLToPath(new URL('..', import.meta.url)),
      env: { ...process.env, GEAR_RETAINED_CUTOVER_FROM_BLOCK: invalid }, encoding: 'utf8',
    });
    assert.notEqual(result.status, 0);
    assert.match(result.stderr, /Invalid GEAR_RETAINED_CUTOVER_FROM_BLOCK/);
  }
});
