import assert from 'node:assert/strict';
import { test } from 'node:test';
import { fileURLToPath } from 'node:url';
import { syncBuiltinESMExports } from 'node:module';

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
const { Transfer, Network, Status, ReceiptRelay, ReceiptSettlement } = await import('../lib/model/index.js');
const quiet = { child() { return this; }, info() {}, debug() {}, warn() {}, error() {} };
const block = { height: 11, hash: hash('aa'), timestamp: 1_700_000_000_000 };
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

test('noncanonical batch fails before effects or cursor-owning handler return', async () => {
  const { receipt, transfer } = outbound();
  const ctx = context(receipt, transfer, hash('cc'));
  await assert.rejects(handleBatch(ctx), /noncanonical Ethereum block/);
  assert.equal(transfer.status, Status.Bridging);
  assert.equal(ctx.saved.length, 0);
});

test('unfinalized batch exhausts bounded wait without persisting any effect', async (t) => {
  t.mock.timers.enable({ apis: ['setTimeout'] });
  syncBuiltinESMExports();
  t.after(() => { t.mock.timers.reset(); syncBuiltinESMExports(); });
  const { receipt, transfer } = outbound();
  const ctx = context(receipt, transfer, block.hash, '0xa');
  const rejection = assert.rejects(handleBatch(ctx), /exceeds canonical finalized/);
  for (let i = 0; i < 4; i++) {
    await new Promise(setImmediate);
    t.mock.timers.tick(30_000);
  }
  await rejection;
  assert.equal(transfer.status, Status.Bridging);
  assert.equal(ctx.saved.length, 0);
});

test('Gear proof envelope and queued native outcome remain pending until exact settled log', async () => {
  const rows = new Map();
  const store = {
    async find(entity, options) {
      return [...(rows.get(entity)?.values() ?? [])].filter((row) =>
        !options?.where || Object.entries(options.where).every(([key, value]) => row[key] === value));
    },
    async findOneBy(entity, where) { return (await this.find(entity, { where }))[0]; },
    async save(items) {
      for (const row of Array.isArray(items) ? items : [items]) {
        if (!rows.has(row.constructor)) rows.set(row.constructor, new Map());
        rows.get(row.constructor).set(row.id, row);
      }
    },
    async remove() {},
  };
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
