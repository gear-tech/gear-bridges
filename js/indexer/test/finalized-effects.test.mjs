import assert from 'node:assert/strict';
import { test } from 'node:test';
import { fileURLToPath } from 'node:url';
import { syncBuiltinESMExports } from 'node:module';
import { spawnSync } from 'node:child_process';

const addr = (byte) => `0x${byte.repeat(20)}`;
const hash = (byte) => `0x${byte.repeat(32)}`;
Object.assign(process.env, {
  ETH_API_PATH: fileURLToPath(new URL('../../../api/ethereum', import.meta.url)),
  ETH_RPC_URL: 'http://127.0.0.1:1', ETH_ERC20_MANAGER: addr('11'), ETH_MSQ_QUEUE: addr('22'),
  ETH_BRIDGING_PAYMENT: addr('33'), ETH_SQUID_API_KEY: 'local-test',
  GEAR_VFT_MANAGER: hash('11'), GEAR_HISTORICAL_PROXY: hash('22'), GEAR_BRIDGING_PAYMENT: hash('33'),
  GEAR_CHECKPOINT_CLIENT: hash('44'), GEAR_SQUID_API_KEY: 'local-test', ETH_HTTP_RPC_URL: 'http://127.0.0.1:1',
});
const { handleBatch } = await import('../lib/eth/handler.js');
const manager = await import('../lib/eth/abi/erc20-manager.js');
const queue = await import('../lib/eth/abi/message-queue.js');
const { BatchState: GearState } = await import('../lib/gear/batch-state.js');
const { requireFinalizedBatch: requireEthFinality } = await import('../lib/eth/finality.js');
const { requireFinalizedBatch: requireGearFinality } = await import('../lib/gear/finality.js');
const { Transfer, CompletedTransfer, MerkleRootInMessageQueue, Network, Status, ReceiptRelay, ReceiptSettlement } = await import('../lib/model/index.js');
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
        if (expected?.type === 'not') return !compare(expected.value);
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
    async findOneBy() { throw new Error('Receipt reconciliation must use batched queries'); },
    async save(items) {
      saves.push(items);
      for (const row of Array.isArray(items) ? items : [items]) {
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
    transactionHash: txHash, blockHash: block.hash, transactionIndex: '0x0' };
}
function outbound(effect = [hash('44'), addr('55'), addr('66'), 10n]) {
  const logs = [event(manager.abi, 'Bridged', effect, addr('11'), 0),
    event(queue.abi, 'MessageProcessed', [9n, hash('77'), 8n, addr('11')], addr('22'), 1)];
  const receipt = { transactionHash: txHash, blockHash: block.hash, blockNumber: '0xb', status: '0x1', logs };
  const transfer = new Transfer({ id: 'original', nonce: '8', sourceNetwork: Network.Vara,
    status: Status.Bridging, bridgingStartedAtBlock: 9n, ethBridgeBuiltInMsgHash: hash('77'),
    sender: hash('44'), receiver: addr('55'), destination: addr('66'), amount: '10' });
  return { receipt, transfer };
}
function context(receipt, transfer, canonicalHash = block.hash, finalizedHeight = '0xb') {
  const saved = [];
  const rpc = {
    async call(method, params) {
      if (method === 'eth_getTransactionReceipt') return receipt;
      return { number: params[0] === 'finalized' ? finalizedHeight : '0xb', hash: block.hash };
    },
    async batchCall() { return [{ number: '0xb', hash: canonicalHash }]; },
  };
  return { _chain: { client: rpc }, log: quiet, saved,
    blocks: [{ header: block, logs: [receipt.logs.at(-1)].map((l) => ({ ...l, logIndex: Number(BigInt(l.logIndex)) })) }],
    store: { async find() { return []; }, async findOneBy() { return transfer; }, async save(rows) { saved.push(...(Array.isArray(rows) ? rows : [rows])); } },
  };
}

test('finalized original same-receipt effect is the only outbound completion', async () => {
  for (const mutate of [
    (r) => { r.logs.shift(); },
    (r) => { r.logs[0] = event(manager.abi, 'Bridged', [hash('44'), addr('55'), addr('66'), 11n], addr('11'), 0); },
    (r) => { r.logs[0].transactionHash = hash('cc'); },
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
    assert.equal(transfer.status, Status.Bridging);
    assert.equal(ctx.saved.length, 0);
  }
  const { receipt, transfer } = outbound();
  const ctx = context(receipt, transfer);
  await handleBatch(ctx);
  assert.equal(transfer.status, Status.Completed);
  assert.equal(transfer.completedAtTxHash, txHash);
  assert.equal(ctx.saved.length, 1);
  const later = outbound();
  later.receipt.logs[1] = event(queue.abi, 'MessageProcessed', [10n, hash('77'), 8n, addr('11')], addr('22'), 1);
  await handleBatch(context(later.receipt, later.transfer));
  assert.equal(later.transfer.status, Status.Completed);
});

test('outbound completion survives counterpart lag and restart while unrelated roots advance', async () => {
  const { receipt, transfer } = outbound();
  const store = memoryStore();
  store.findOneBy = async (entity, where) => (await store.find(entity, { where }))[0];
  const ctx = { ...context(receipt, undefined), store };
  ctx.blocks[0].logs.push({ ...event(queue.abi, 'MerkleRoot', [12n, hash('88')], addr('22'), 2), logIndex: 2 });
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
  assert.equal(transfer.status, Status.Completed);
  assert.equal(transfer.completedAtTxHash, txHash);
  assert.equal(transfer.completedAtBlock, 11n);
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
  await assert.rejects(handleBatch(changed), /noncanonical Ethereum block/);
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
  await assert.rejects(handleBatch(ctx), /noncanonical Ethereum block/);
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

test('Ethereum finality lag stays pending beyond 120 seconds with no effects, then completes canonically', async (t) => {
  mockWait(t);
  const { receipt, transfer } = outbound();
  const ctx = context(receipt, transfer);
  const rpc = ctx._chain.client;
  let polls = 0, batches = 0, finished = false;
  ctx._chain.client = {
    async call(method, params) {
      if (method === 'eth_getBlockByNumber') {
        return { number: params[0] === 'finalized' ? (++polls <= 5 ? '0xa' : '0xb') : params[0], hash: block.hash };
      }
      return rpc.call(method, params);
    },
    async batchCall(calls) { batches++; return rpc.batchCall(calls); },
  };
  const pending = handleBatch(ctx).then(() => { finished = true; });
  for (let i = 0; i < 4; i++) {
    await new Promise(setImmediate);
    t.mock.timers.tick(30_000);
  }
  await new Promise(setImmediate);
  assert.equal(polls, 5);
  assert.equal(finished, false);
  assert.equal(batches, 0);
  assert.equal(transfer.status, Status.Bridging);
  assert.equal(ctx.saved.length, 0);
  t.mock.timers.tick(29_999);
  await new Promise(setImmediate);
  assert.equal(finished, false);
  t.mock.timers.tick(1);
  await pending;
  assert.equal(polls, 6);
  assert.equal(transfer.status, Status.Completed);
  assert.equal(ctx.saved.length, 1);
});

test('Gear finality lag holds the entire batch beyond 120 seconds before any state access', async (t) => {
  mockWait(t);
  const ctx = finalityContext('Gear', [10, 10, 10, 10, 10, 11]);
  const store = memoryStore(), state = new GearState();
  Object.assign(ctx, { store, log: quiet });
  let finished = false;
  const pending = requireGearFinality(ctx).then(async () => {
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

for (const [chain, requireFinality] of [['Ethereum', requireEthFinality], ['Gear', requireGearFinality]]) {
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
  const transfer = new Transfer({ id: 'deposit', sourceNetwork: Network.Ethereum, blockNumber: 100n,
    sourceTransactionIndex: 2n, sourceLogIndex: 3n, source: addr('66'), destination: hash('77'),
    sender: addr('55'), receiver: hash('44'), amount: '10', status: Status.Bridging });
  await store.save(transfer);
  const state = new GearState();
  const ctx = { store, log: quiet };
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
  assert.equal(another.status, Status.Completed);
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
    const store = memoryStore(), state = new GearState(), ctx = { store, log: quiet };
    const transfer = new Transfer({ id: 'original', sourceNetwork: Network.Ethereum, blockNumber: 100n,
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
    assert.equal(transfer.status, 'Refunded');
    assert.notEqual(transfer.status, Status.Completed);
    assert.equal(transfer.receiptSlot, 200n);
    assert.equal(transfer.completedAtBlock, 300n);
    assert.equal(transfer.completedAtTxHash, hash('dd'));
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
      assert.equal(transfer.status, Status.Refunded);
    }
    await state.new(ctx);
    await dispatch(state, deposit, 'ReceiptDepositSettled');
    await assert.rejects(state.save(), /conflicting receipt deposit/);
    assert.equal(transfer.status, Status.Refunded);
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
  const state = new GearState(), ctx = { store, log: quiet };
  await state.new(ctx);
  await state.save();
  assert.equal(store.queries.find(({ entity, options }) => entity === ReceiptSettlement && options.where?.matched === false).options.take, 1000);
  const transfer = new Transfer({ id: 'late', sourceNetwork: Network.Ethereum, blockNumber: 100n,
    sourceTransactionIndex: 2n, sourceLogIndex: 3n, source: addr('66'), destination: hash('77'),
    sender: addr('55'), receiver: hash('44'), amount: '10', status: Status.Bridging });
  await store.save([transfer]);
  await state.new(ctx);
  state.recordReceiptRelay({ slot: '1200', block_number: 100, transaction_index: 2 });
  const before = store.queries.length;
  await state.save();
  assert.equal(transfer.status, Status.Completed);
  assert.equal(pending[1000].matched, true);
  assert.equal(pending[0].matched, false);
  assert.ok(store.queries.length - before <= 6);
  const first = new Transfer({ ...transfer, id: 'first', blockNumber: 101n, receiptSlot: null, status: Status.Bridging });
  await store.save([first]);
  await state.new(ctx);
  state.recordReceiptRelay({ slot: '200', block_number: 101, transaction_index: 2 });
  await state.save();
  assert.equal(first.status, Status.Completed);
  assert.equal(pending[0].matched, true);
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
    const transfer = new Transfer({ id: 'original', sourceNetwork: Network.Ethereum, blockNumber: 100n,
      sourceTransactionIndex: 2n, sourceLogIndex: 3n, source: addr('66'), destination: hash('77'),
      sender: addr('55'), receiver: hash('44'), amount: '10', status: Status.Bridging });
    if (field === 'sourceNetwork') transfer[field] = Network.Vara;
    else if (['blockNumber', 'sourceTransactionIndex', 'sourceLogIndex', 'receiptSlot'].includes(field)) transfer[field] = 999n;
    else if (field !== 'duplicate') transfer[field] = field === 'amount' ? '11' : hash('99');
    await store.save([transfer, ...(field === 'duplicate' ? [new Transfer({ ...transfer, id: 'duplicate' })] : [])]);
    const state = new GearState();
    await state.new({ store, log: quiet });
    state.recordReceiptRelay({ slot: '200', block_number: 100, transaction_index: 2 });
    state.recordReceiptSettlement(deposit, new Date(), 300n, hash('bb'));
    if (field === 'receiptSlot' || field === 'duplicate') await assert.rejects(state.save(), /HOLD:/);
    else {
      await state.save();
      assert.equal((await store.find(ReceiptSettlement))[0].matched, false, field);
    }
    assert.equal(transfer.status, Status.Bridging, field);
  }
  const store = memoryStore(), state = new GearState(), ctx = { store, log: quiet };
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
