import assert from 'node:assert/strict';
import { test } from 'node:test';
import { spawn } from 'node:child_process';
import { once } from 'node:events';
import { readdir } from 'node:fs/promises';
import { createServer } from 'node:net';
import { fileURLToPath } from 'node:url';
import pg from 'pg';
import { createEvidenceFixture, fixtureIdentity as identity, hash } from './evidence-fixture.mjs';

const cwd = fileURLToPath(new URL('..', import.meta.url));
const migrationFile = '1792540800000-ethereum_evidence.js';
const nativeMigrationFile = '1792627200000-native_ethereum_evidence.js';
const tables = ['transfer', 'completed_transfer', 'receipt_settlement', 'receipt_relay', 'gear_manager_handoff',
  'pair', 'initiated_transfer', 'gear_eth_bridge_message', 'vara_bridge_program', 'eth_bridge_program',
  'merkle_root_in_message_queue', 'ethereum_finality_checkpoint', 'migrations'];

// Human-readable fixture labels map to real UUID transfer IDs accepted by the CLI.
const labels = ['00-pending', '10-direct', '15-missing', '20-retained', '30-near', '40-inbound', '50-refund',
  '60-awaiting', '61-bridging', '62-failed', '63-other-direction', '64-eth-failed'];
const ids = new Map(labels.map((label, index) => [label, `00000000-0000-4000-8000-${String(index).padStart(12, '0')}`]));
const fixtureId = (label) => ids.get(label) ?? label;
const fixtureLabel = (id) => labels.find((label) => ids.get(label) === id) ?? id;

function disposableUrl() {
  assert.ok(process.env.INDEXER_TEST_DATABASE_URL, 'INDEXER_TEST_DATABASE_URL is required; refusing to skip the real database smoke');
  const url = new URL(process.env.INDEXER_TEST_DATABASE_URL);
  assert.ok(['postgres:', 'postgresql:'].includes(url.protocol), 'a PostgreSQL URL is required');
  assert.ok(['127.0.0.1', 'localhost', '[::1]'].includes(url.hostname), 'only an inspected loopback disposable database is permitted');
  assert.match(decodeURIComponent(url.pathname.slice(1)), /^(?:gear_bridges_phase1_evidence|indexer_evidence_[a-z0-9_]+)$/,
    'database must be the explicitly inspected phase-one database or have a task-owned indexer_evidence_ prefix');
  assert.equal(url.search, '', 'connection overrides are forbidden');
  assert.ok(url.username, 'explicit task database owner is required');
  return url;
}

function launch(file, args, env, timeout = 45_000) {
  const child = spawn(process.execPath, [file, ...args], { cwd, env, stdio: ['ignore', 'pipe', 'pipe'] });
  let stdout = '', stderr = '';
  child.stdout.on('data', (chunk) => { stdout += chunk; });
  child.stderr.on('data', (chunk) => { stderr += chunk; });
  const timer = setTimeout(() => child.kill('SIGKILL'), timeout);
  const done = new Promise((resolve, reject) => {
    child.once('error', reject);
    child.once('close', (code, signal) => resolve({ code, signal, stdout, stderr }));
  }).finally(() => clearTimeout(timer));
  return { child, done, output: () => ({ stdout, stderr }) };
}

async function freePort() {
  const server = createServer();
  server.listen(0, '127.0.0.1');
  await once(server, 'listening');
  const port = server.address().port;
  await new Promise((resolve) => server.close(resolve));
  return port;
}

async function graphQL(port) {
  const response = await fetch(`http://127.0.0.1:${port}/graphql`, { method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify({ query: '{ transfers(first: 100) { id status completedAt completedAtBlock completedAtTxHash } }' }),
    signal: AbortSignal.timeout(2_000),
  });
  assert.equal(response.status, 200);
  const body = await response.json();
  assert.equal(body.errors, undefined, JSON.stringify(body.errors));
  return new Map(body.data.transfers.map((row) => [fixtureLabel(row.id), row]));
}

// This suite deliberately leaves its inspected task-owned database in place for the standalone repair smoke.
// It never drops a database/schema and refuses to touch a populated database on a second invocation.
test('real migration, bounded reconciliation CLI and API retain authenticated original evidence', { timeout: 180_000 }, async (t) => {
  const url = disposableUrl();
  const db = new pg.Client({ connectionString: url.href });
  await db.connect();
  t.after(() => db.end());
  const ownership = (await db.query(`SELECT current_database() AS name, current_user AS owner,
    pg_get_userbyid(datdba) AS database_owner FROM pg_database WHERE datname = current_database()`)).rows[0];
  assert.equal(ownership.name, decodeURIComponent(url.pathname.slice(1)));
  assert.equal(ownership.owner, ownership.database_owner, 'the connecting test role must own the disposable database');
  const existing = await db.query(`SELECT n.nspname, c.relname FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace
    WHERE n.nspname NOT IN ('pg_catalog', 'information_schema') AND n.nspname NOT LIKE 'pg_toast%'
      AND c.relkind IN ('r','p','v','m','f')`);
  assert.deepEqual(existing.rows, [], 'refusing to modify a nonempty database; provision a fresh task-owned database');
  const types = await db.query(`SELECT typname FROM pg_type WHERE typnamespace = 'public'::regnamespace AND typtype = 'e'`);
  assert.deepEqual(types.rows, [], 'refusing a preexisting application schema');

  const fixture = await createEvidenceFixture();
  t.after(() => fixture.close());
  const env = { ...process.env, ...fixture.env, DATABASE_URL: url.href, DB_URL: url.href,
    DB_HOST: url.hostname.replace(/[\[\]]/g, ''), DB_PORT: url.port || '5432', DB_USER: decodeURIComponent(url.username),
    DB_PASS: decodeURIComponent(url.password), DB_NAME: ownership.name, NODE_ENV: 'test' };
  const run = async (direction, args = [], success = true, freshArchive = false) => {
    // Fresh negative-test owners must fetch proofs; retained successful archives are never discarded.
    const runtime = freshArchive ? { ...env, ETH_FINALITY_ARCHIVE_ROOT: await fixture.freshArchiveRoot() } : env;
    const result = await launch('lib/eth/reconcile.js', ['--direction', direction, ...args.map(fixtureId)], runtime).done;
    if (!success) {
      assert.notEqual(result.code, 0, `unsafe repair succeeded: ${result.stdout}`);
      assert.ok(!result.stdout.includes('lastExaminedId'), 'failed page must not emit a committed success cursor');
      return result;
    }
    assert.equal(result.code, 0, result.stderr || result.stdout);
    const line = result.stdout.trim().split('\n').findLast((value) => value.startsWith('{') && value.includes('lastExaminedId'));
    assert.ok(line, `missing repair report: ${result.stdout}`);
    const report = JSON.parse(line);
    report.lastExaminedId = fixtureLabel(report.lastExaminedId);
    return report;
  };
  const snapshot = async () => Object.fromEntries(await Promise.all(tables.map(async (table) => [table,
    (await db.query(`SELECT COALESCE(jsonb_agg(to_jsonb(t) ORDER BY id), '[]'::jsonb)::text AS value FROM "${table}" t`)).rows[0].value])));
  const transfer = async (id) => (await db.query('SELECT * FROM transfer WHERE id = $1', [fixtureId(id)])).rows[0];
  const { DataSource } = await import('typeorm');
  const migrations = (await readdir(new URL('../db/migrations/', import.meta.url))).filter((file) => file.endsWith('.js')).sort();
  assert.ok(migrations.includes(migrationFile), 'the real evidence migration must exist');
  assert.ok(migrations.includes(nativeMigrationFile), 'the forward native-evidence migration must exist');
  const migrate = async (method = 'up', beforeFile) => {
    const selected = migrations.filter((file) => !beforeFile || file < beforeFile);
    const classes = await Promise.all(selected.map(async (name) =>
      (await import(new URL(`../db/migrations/${name}`, import.meta.url))).default));
    const database = new DataSource({ type: 'postgres', url: url.href, migrations: classes,
      synchronize: false, migrationsRun: false, logging: false });
    try {
      await database.initialize();
      if (method === 'down') await database.undoLastMigration({ transaction: 'all' });
      else await database.runMigrations({ transaction: 'all' });
    } finally {
      if (database.isInitialized) await database.destroy();
    }
  };
  await migrate('up', migrationFile);

  const insert = async (table, row) => {
    if (table === 'transfer') row = { ...row, id: fixtureId(row.id) };
    const keys = Object.keys(row);
    await db.query(`INSERT INTO "${table}" (${keys.map((key) => `"${key}"`).join(',')}) VALUES (${keys.map((_, i) => `$${i + 1}`).join(',')})`, Object.values(row));
  };
  const time = '2023-11-14T22:13:31.123Z';
  const base = { tx_hash: hash('c0'), block_number: '9', timestamp: time, nonce: '8', source_network: 'Vara',
    source: identity.varaToken, dest_network: 'Ethereum', destination: identity.token, status: 'Completed',
    sender: identity.sender, receiver: identity.receiver, amount: '10', bridging_started_at_block: '9',
    eth_bridge_built_in_msg_hash: identity.message, completed_at: '2023-11-14T22:13:31.123456Z', completed_at_block: '11',
    completed_at_tx_hash: fixture.txHashes[0] };
  const inbound = { ...base, source_network: 'Ethereum', dest_network: 'Vara', source: identity.token,
    destination: identity.varaToken, sender: identity.receiver, receiver: identity.sender, block_number: '11',
    source_log_index: null, completed_at_block: '300', completed_at_tx_hash: hash('d0') };
  const seeds = [
    { ...base, id: '00-pending', status: 'Bridging', nonce: '700', completed_at: null, completed_at_block: null, completed_at_tx_hash: null },
    { ...base, id: '10-direct' },
    { ...base, id: '20-retained', nonce: '9', completed_at_tx_hash: fixture.txHashes[1] },
    { ...base, id: '30-near', nonce: '888', eth_bridge_built_in_msg_hash: hash('99') },
    { ...inbound, id: '40-inbound', tx_hash: fixture.txHashes[2], source_transaction_index: null },
    { ...inbound, id: '50-refund', tx_hash: fixture.txHashes[3], source_transaction_index: null, status: 'Refunded' },
    { ...base, id: '60-awaiting', status: 'AwaitingPayment', nonce: '701', completed_at: null, completed_at_block: null, completed_at_tx_hash: null },
    { ...base, id: '61-bridging', status: 'Bridging', nonce: '702', completed_at: null, completed_at_block: null, completed_at_tx_hash: null },
    { ...base, id: '62-failed', status: 'Failed', nonce: '703' },
    { ...base, id: '63-other-direction', dest_network: 'Vara', nonce: '704' },
    { ...inbound, id: '64-eth-failed', status: 'Failed', tx_hash: hash('c4'), source_transaction_index: '4' },
  ];
  for (const row of seeds) await insert('transfer', row);
  const oldReceipt = structuredClone(fixture.receipts[1]);
  for (const field of ['transactionIndex', 'type', 'cumulativeGasUsed', 'logsBloom']) delete oldReceipt[field];
  for (const log of oldReceipt.logs) { delete log.blockNumber; delete log.transactionIndex; }
  const outbound = { receipt: oldReceipt,
    logIndex: Number(BigInt(oldReceipt.logs.find((log) => log.address === identity.queue).logIndex)),
    nonce: '9', queue: identity.queue, manager: identity.manager };
  await insert('completed_transfer', { id: `outbound:${identity.queue}:9`, src_network: 'Vara', dest_network: 'Ethereum',
    tx_hash: fixture.txHashes[1], block_number: '11', timestamp: time, outbound_receipt: outbound, matched: true });
  await insert('completed_transfer', { id: 'legacy-unmatched', src_network: 'Vara', dest_network: 'Ethereum',
    tx_hash: fixture.txHashes[1], block_number: '11', timestamp: time,
    outbound_receipt: { ...outbound, nonce: '999' }, matched: false });
  await insert('completed_transfer', { id: 'unrelated-completion', src_network: 'Ethereum', dest_network: 'Vara',
    tx_hash: hash('e0'), block_number: '301', timestamp: time, matched: true });
  for (const [index, slot] of [[2, '200'], [3, '201']]) {
    await insert('receipt_relay', { id: `${slot}:${index}`, block_number: '11' });
    const deposit = { slot, transaction_index: String(index), log_index: '0', deposit_count: '1', operation_id: hash('e1'),
      eth_token_id: identity.token, vara_token_id: identity.varaToken, sender: identity.receiver, receiver: identity.sender,
      amount: '10', native: true, ...(index === 3 ? { outcome: 'Refunded', child: hash('e2'), payout_child: hash('e3'), returned_value: '10' } : {}) };
    await insert('receipt_settlement', { id: `${slot}:${index}:0`, deposit, timestamp: time, block_number: '300', tx_hash: hash('d0'), matched: true });
  }
  const before = new Map((await db.query('SELECT * FROM transfer ORDER BY id')).rows.map((row) => [fixtureLabel(row.id), row]));
  const depositsBefore = (await db.query('SELECT id, deposit FROM receipt_settlement ORDER BY id')).rows;
  await migrate('up', nativeMigrationFile);

  await t.test('migration immediately preserves and invalidates only unauthenticated projections', async () => {
    const affected = new Set(['10-direct', '20-retained', '30-near', '40-inbound', '50-refund']);
    for (const [id, original] of before) {
      const row = await transfer(id);
      const { source_receipt_auth, completion_auth, legacy_resolution, ...unchanged } = row;
      assert.equal(source_receipt_auth, null); assert.equal(completion_auth, null);
      if (!affected.has(id)) { assert.equal(legacy_resolution, null); assert.deepEqual(unchanged, original); continue; }
      assert.equal(row.status, 'Bridging');
      for (const field of ['completed_at', 'completed_at_block', 'completed_at_tx_hash']) assert.equal(row[field], null);
      assert.equal(legacy_resolution.status, original.status);
      assert.equal(legacy_resolution.completedAtBlock, original.completed_at_block);
      assert.equal(legacy_resolution.completedAtTxHash, original.completed_at_tx_hash);
      assert.match(legacy_resolution.completedAt, /\.123456/, 'preserve submillisecond SQL timestamp precision');
      for (const [key, value] of Object.entries(original)) {
        if (!['status', 'completed_at', 'completed_at_block', 'completed_at_tx_hash'].includes(key)) assert.deepEqual(row[key], value, `${id}.${key}`);
      }
    }
    const retained = (await db.query('SELECT * FROM completed_transfer WHERE id = $1', [`outbound:${identity.queue}:9`])).rows[0];
    assert.equal(retained.matched, false); assert.equal(retained.legacy_matched, true);
    assert.deepEqual(retained.outbound_receipt, outbound); assert.equal(retained.receipt_auth, null);
    const unmatched = (await db.query("SELECT * FROM completed_transfer WHERE id = 'legacy-unmatched'")).rows[0];
    assert.equal(unmatched.matched, false); assert.equal(unmatched.legacy_matched, false);
    assert.equal(unmatched.receipt_auth, null); assert.deepEqual(unmatched.outbound_receipt, { ...outbound, nonce: '999' });
    const other = (await db.query("SELECT * FROM completed_transfer WHERE id = 'unrelated-completion'")).rows[0];
    assert.equal(other.matched, true); assert.equal(other.legacy_matched, null);
    assert.deepEqual((await db.query('SELECT id, deposit FROM receipt_settlement ORDER BY id')).rows, depositsBefore);
    assert.ok((await db.query('SELECT matched, legacy_matched FROM receipt_settlement')).rows.every((row) => !row.matched && row.legacy_matched));
    const saved = await snapshot();
    await assert.rejects(migrate('down', nativeMigrationFile), /Cannot|preserv|evidence/i);
    assert.deepEqual(await snapshot(), saved, 'rollback refusal is atomic');
  });

  const legacyHeader = fixture.headers[11];
  const legacyCheckpoint = { schemaVersion: 1, policy: 'independent-two-el-v1',
    chainId: '31337', genesisHash: fixture.headers[0].hash, number: '11', timestamp: '1700000011',
    hash: legacyHeader.hash, parentHash: legacyHeader.parentHash, stateRoot: legacyHeader.stateRoot,
    transactionsRoot: legacyHeader.transactionsRoot, receiptsRoot: legacyHeader.receiptsRoot };
  const legacyCheckpointId = `${legacyCheckpoint.chainId}:${legacyCheckpoint.genesisHash}:11`;
  await insert('ethereum_finality_checkpoint', { id: legacyCheckpointId, block_number: '11', checkpoint: legacyCheckpoint });
  for (const [label, index] of [['10-direct', 0], ['20-retained', 1], ['40-inbound', 2], ['50-refund', 3]]) {
    const source = index >= 2 ? { version: 1, checkpointId: legacyCheckpointId, receipt: fixture.receipts[index],
      blockTimestamp: '1700000011', localLogIndex: 0 } : null;
    const completion = index < 2 ? { version: 1, kind: 'ethereum-receipt', checkpointId: legacyCheckpointId,
      completionId: `outbound:${identity.queue}:${8 + index}`,
      processedLogIndex: Number(BigInt(fixture.receipts[index].logs.find((log) => log.address === identity.queue).logIndex)) }
      : { version: 1, kind: 'gear-settlement', sourceCheckpointId: legacyCheckpointId,
        receiptRelayId: `${198 + index}:${index}`, receiptSettlementId: `${198 + index}:${index}:0`,
        outcome: index === 3 ? 'Refunded' : 'Completed' };
    const original = before.get(label);
    await db.query(`UPDATE transfer SET source_receipt_auth=$2, completion_auth=$3, status=$4,
      completed_at=$5, completed_at_block=$6, completed_at_tx_hash=$7 WHERE id=$1`,
    [fixtureId(label), source, completion, original.status, '2023-11-14T22:13:31.123456Z',
      original.completed_at_block, original.completed_at_tx_hash]);
  }
  const retainedReceiptAuth = { version: 1, checkpointId: legacyCheckpointId,
    receipt: fixture.receipts[1], blockTimestamp: '1700000011' };
  await db.query('UPDATE completed_transfer SET receipt_auth=$2, matched=true WHERE id=$1',
    [`outbound:${identity.queue}:9`, retainedReceiptAuth]);
  await db.query("UPDATE completed_transfer SET receipt_auth=$1 WHERE id='legacy-unmatched'", [retainedReceiptAuth]);
  await db.query('UPDATE receipt_settlement SET matched=true');
  const preNative = await snapshot();
  await migrate();
  const nativeMigrated = await snapshot();

  await t.test('native migration archives old authorizations without rewriting prior history or checkpoint identity', async () => {
    const originals = new Map(JSON.parse(preNative.transfer).map((row) => [row.id, row]));
    for (const label of ['10-direct', '20-retained', '40-inbound', '50-refund']) {
      const row = await transfer(label), old = originals.get(row.id);
      assert.deepEqual(row.legacy_ethereum_auth.sourceReceiptAuth, old.source_receipt_auth);
      assert.deepEqual(row.legacy_ethereum_auth.completionAuth, old.completion_auth);
      assert.equal(row.legacy_ethereum_auth.status, old.status);
      assert.match(row.legacy_ethereum_auth.completedAt, /\.123456/);
      assert.equal(row.legacy_ethereum_auth.completedAtBlock, String(old.completed_at_block));
      assert.equal(row.legacy_ethereum_auth.completedAtTxHash, old.completed_at_tx_hash);
      assert.deepEqual(row.legacy_resolution, old.legacy_resolution, 'never overwrite the earlier terminal projection');
      assert.equal(row.source_receipt_auth, null); assert.equal(row.completion_auth, null);
      assert.equal(row.status, 'Bridging');
      for (const field of ['completed_at', 'completed_at_block', 'completed_at_tx_hash']) assert.equal(row[field], null);
    }
    for (const label of ['00-pending', '30-near', '60-awaiting', '61-bridging', '62-failed', '63-other-direction', '64-eth-failed']) {
      const { legacy_ethereum_auth, ...row } = await transfer(label);
      assert.equal(legacy_ethereum_auth, null);
      // JSONB timestamps/bigints are normalized by PostgreSQL, not by the driver.
      assert.deepEqual((await db.query("SELECT to_jsonb(t) - 'legacy_ethereum_auth' AS row FROM transfer t WHERE id=$1", [row.id])).rows[0].row,
        originals.get(row.id));
    }
    const retained = (await db.query('SELECT * FROM completed_transfer WHERE id=$1', [`outbound:${identity.queue}:9`])).rows[0];
    assert.deepEqual(retained.legacy_receipt_auth.receiptAuth, retainedReceiptAuth);
    assert.equal(retained.legacy_receipt_auth.matched, true);
    assert.equal(retained.legacy_receipt_auth.blockNumber, '11');
    assert.equal(retained.legacy_receipt_auth.txHash, fixture.txHashes[1]);
    assert.equal(Date.parse(retained.legacy_receipt_auth.timestamp), Date.parse(time));
    assert.equal(retained.receipt_auth, null); assert.equal(retained.matched, false); assert.equal(retained.legacy_matched, true);
    assert.deepEqual(retained.outbound_receipt, outbound);
    const unmatched = (await db.query("SELECT * FROM completed_transfer WHERE id='legacy-unmatched'")).rows[0];
    assert.deepEqual(unmatched.legacy_receipt_auth.receiptAuth, retainedReceiptAuth);
    assert.equal(unmatched.legacy_receipt_auth.matched, false);
    assert.equal(unmatched.legacy_matched, false); assert.equal(unmatched.matched, false); assert.equal(unmatched.receipt_auth, null);
    assert.deepEqual((await db.query('SELECT id, deposit FROM receipt_settlement ORDER BY id')).rows, depositsBefore);
    assert.ok((await db.query('SELECT matched, legacy_matched FROM receipt_settlement')).rows.every((row) => !row.matched && row.legacy_matched));
    const after = await snapshot();
    assert.equal(after.ethereum_finality_checkpoint, preNative.ethereum_finality_checkpoint);
    assert.equal(after.receipt_relay, preNative.receipt_relay);
    assert.equal(after.gear_manager_handoff, preNative.gear_manager_handoff);
    const NativeMigration = (await import(`../db/migrations/${nativeMigrationFile}`)).default;
    await assert.rejects(db.query('BEGIN').then(async () => {
      try { await new NativeMigration().down({ query: (sql) => db.query(sql) }); }
      finally { await db.query('ROLLBACK'); }
    }), /Cannot|preserv|evidence/i);
    assert.deepEqual(await snapshot(), after, 'archive-loss refusal cannot restore unqualified completion');
  });

  const port = await freePort();
  const api = launch('lib/api.js', [], { ...env, GQL_HOST: '127.0.0.1', GQL_PORT: String(port) }, 180_000);
  t.after(async () => { api.child.kill('SIGTERM'); await api.done; });
  let apiRows;
  const deadline = Date.now() + 15_000;
  while (Date.now() < deadline) {
    try { apiRows = await graphQL(port); break; }
    catch (error) {
      if (api.child.exitCode !== null) throw new Error(`API exited: ${JSON.stringify(api.output())}`, { cause: error });
      await new Promise((resolve) => setTimeout(resolve, 100));
    }
  }
  assert.ok(apiRows, `API failed to become ready: ${JSON.stringify(api.output())}`);
  for (const id of ['10-direct', '20-retained', '30-near', '40-inbound', '50-refund']) {
    assert.equal(apiRows.get(id).completedAt, null);
    assert.equal(apiRows.get(id).status.toLowerCase(), 'bridging', 'existing API claim predicate remains visible');
  }

  await t.test('legacy backlog missing historical receipts does not wedge live SQL batches or certify operator skips', async () => {
    const localEnv = { ...fixture.env, ETH_RPC_ONLY: 'true', ETH_BRIDGING_PAYMENT: `0x${'33'.repeat(20)}`,
      GEAR_RPC_ONLY: 'true', GEAR_API_PATH: fileURLToPath(new URL('../../../api/gear', import.meta.url)),
      GEAR_VFT_MANAGER: hash('11'), GEAR_HISTORICAL_PROXY: hash('22'), GEAR_BRIDGING_PAYMENT: hash('33'),
      GEAR_CHECKPOINT_CLIENT: hash('44'), ETH_HTTP_RPC_URL: fixture.env.ETH_RPC_URL };
    const oldEnv = Object.fromEntries(Object.keys(localEnv).map((key) => [key, process.env[key]]));
    Object.assign(process.env, localEnv);
    await import('reflect-metadata');
    const { Store } = await import('@subsquid/typeorm-store');
    const { default: configured } = await import('../lib/model/dataSource.js');
    const { handleBatch } = await import('../lib/eth/handler.js');
    const { BatchState: GearState } = await import('../lib/gear/batch-state.js');
    const { createEthereumEvidenceClients, destroyEthereumEvidenceClients } = await import('../lib/eth/finality.js');
    const database = new DataSource({ ...configured.options, url: url.href, synchronize: false,
      migrationsRun: false, dropSchema: false, logging: false });
    const absentHash = hash('ce');
    await db.query(`INSERT INTO completed_transfer
      (id, src_network, dest_network, tx_hash, block_number, timestamp, outbound_receipt, matched, legacy_matched)
      SELECT 'legacy-backlog-' || lpad(n::text, 4, '0'), 'Vara', 'Ethereum', $1::text, 11, $2,
        jsonb_set(jsonb_set($3::jsonb, '{nonce}', to_jsonb((1000+n)::text)),
          '{receipt,transactionHash}', to_jsonb($1::text)), false, n % 2 = 0
      FROM generate_series(1, 1001) n`, [absentHash, time, outbound]);
    const legacyRows = (await db.query('SELECT * FROM completed_transfer ORDER BY id')).rows;
    assert.ok(legacyRows.filter((row) => row.legacy_matched != null && row.receipt_auth == null).length > 1000);
    const legacyTransfer = await transfer('20-retained');
    const lateTransfer = await transfer('10-direct');
    const freshId = `outbound:${identity.queue}:8`;
    const root = fixture.receipts[0].logs.at(-1);
    assert.equal(root.logIndex, '0x2', 'the MerkleRoot event must be signed, never appended after signing');
    const quiet = { child() { return this; }, info() {}, debug() {}, warn() {}, error() {} };
    const live = () => database.transaction('SERIALIZABLE', async (manager) => {
      const clients = createEthereumEvidenceClients('eth');
      await handleBatch({ store: new Store(() => manager), log: quiet,
        _chain: { client: { call: clients.primaryCall } },
        blocks: [{ header: { height: 11, hash: fixture.headers[11].hash, timestamp: 1_700_000_011_000 },
          logs: [{ ...root, logIndex: 2, transactionIndex: 0 }] }] });
    });
    let parkedSettlement;
    try {
      await database.initialize();
      // Reconstructed from HTML7fe1b032 VB-41/F8: the real SQL transaction,
      // not a callback save, owns role/effect/checkpoint atomicity.
      const roles = await import('../lib/gear/programIds.js');
      const { Decoder } = await import('../lib/gear/codec.js');
      const { handleVftManagerEvents } = await import('../lib/gear/handlers/vft-manager.js');
      const { GearProgram, InitiatedTransfer } = await import('../lib/model/index.js');
      const { Sails } = await import('sails-js');
      const { SailsIdlParser } = await import('sails-js-parser');
      const { readFile } = await import('node:fs/promises');
      const parser = new SailsIdlParser();
      await parser.init();
      const sails = new Sails(parser).parseIdl(await readFile(new URL('../../../api/gear/vft_manager.idl', import.meta.url), 'utf8'));
      const decoder = new Decoder(sails), nextProxy = hash('66');
      await db.query(`INSERT INTO gear_programs(name, program_id) VALUES
        ('vft_manager',$1),('historical_proxy',$2),('bridging_payment',$3)`, [hash('11'), hash('22'), hash('33')]);
      const beforeRoles = (await db.query('SELECT * FROM gear_programs ORDER BY name')).rows;
      const beforeBoundary = await snapshot();
      const rotate = async (store, state) => {
        await roles.reloadPrograms(store);
        await state.new({ store, log: quiet, blocks: [] });
        const method = 'HistoricalProxyAddressChanged';
        const payload = sails.registry.createType(`(String,String,${JSON.stringify(sails.services.VftManager.events[method].type)})`,
          ['VftManager', method, { old: hash('22'), new: nextProxy }]).toHex();
        await handleVftManagerEvents({ state, decoder, service: 'VftManager', method, log: quiet,
          blockHeader: { height: 300 }, event: { args: { message: { payload } } } });
      };
      await assert.rejects(database.transaction('SERIALIZABLE', async (manager) => {
        const store = new Store(() => manager), staged = new GearState();
        await rotate(store, staged);
        await staged.addInitiatedTransfer(new InitiatedTransfer({ id: hash('ab'), txHash: hash('ac'), blockNumber: 300n }));
        await staged.save(); // This must NOT commit anything outside this transaction.
        await handleVftManagerEvents({ state: staged, decoder, service: 'VftManager', method: 'ReceiptDepositSettled',
          log: quiet, blockHeader: { height: 300 }, event: { args: { message: { payload: '0xff' } } } });
      }), /Invalid.*payload/);
      assert.deepEqual(await snapshot(), beforeBoundary, 'trusted decode failure rolls back effects and SQL checkpoints');
      assert.deepEqual((await db.query('SELECT * FROM gear_programs ORDER BY name')).rows, beforeRoles);
      await database.transaction('SERIALIZABLE', async (manager) => {
        const store = new Store(() => manager);
        await roles.reloadPrograms(store);
        assert.equal(roles.programs.get(hash('22')), 'historical_proxy', 'retry discards tentative in-memory rotation');
        await rotate(store, new GearState());
      });
      assert.equal((await db.query("SELECT program_id FROM gear_programs WHERE name='historical_proxy'")).rows[0].program_id, nextProxy);
      const failStore = { save: async () => { throw new Error('SQL write refused'); } };
      await assert.rejects(roles.updateId(failStore, 'historical_proxy', hash('77')), /SQL write refused/);
      assert.equal(roles.programs.get(nextProxy), 'historical_proxy', 'failed store cannot advance memory');
      assert.equal((await database.getRepository(GearProgram).findOneByOrFail({ id: 'historical_proxy' })).programId, nextProxy);
      fixture.state.fault = ({ method, result }) => method === 'eth_getBlockReceipts' ? null : result;
      const beforeFailure = await snapshot();
      await assert.rejects(live(), /HOLD|unavailable|receipt/i);
      assert.deepEqual(await snapshot(), beforeFailure, 'current-batch qualification remains mandatory and atomic');
      destroyEthereumEvidenceClients('eth');
      fixture.state.fault = undefined;
      await db.query('UPDATE transfer SET nonce=$2 WHERE id=$1', [lateTransfer.id, '8000']);
      await insert('completed_transfer', { id: freshId, src_network: 'Vara', dest_network: 'Ethereum',
        tx_hash: fixture.txHashes[0], block_number: '11', timestamp: time, matched: false,
        outbound_receipt: { receipt: fixture.receipts[0], logIndex: 1, nonce: '8', queue: identity.queue, manager: identity.manager } });
      fixture.state.requests.length = 0;
      await live();
      assert.equal((await db.query('SELECT count(*)::int AS count FROM merkle_root_in_message_queue')).rows[0].count, 1);
      const qualified = (await db.query('SELECT * FROM completed_transfer WHERE id=$1', [freshId])).rows[0];
      assert.equal(qualified.receipt_auth.version, 2);
      assert.match(qualified.receipt_auth.inclusionProofDigest, /^0x[0-9a-f]{64}$/);
      assert.equal(qualified.matched, false, 'fresh receipt qualifies beyond the repair-owned backlog before its counterpart arrives');
      await db.query('UPDATE completed_transfer SET legacy_matched=true WHERE id=$1', [freshId]);
      await db.query('UPDATE transfer SET nonce=$2 WHERE id=$1', [lateTransfer.id, lateTransfer.nonce]);
      await live();
      const matched = (await db.query('SELECT * FROM completed_transfer WHERE id=$1', [freshId])).rows[0];
      assert.equal(matched.matched, true, 'qualified late counterpart remains eligible even with a legacy marker');
      assert.deepEqual(matched.receipt_auth, qualified.receipt_auth, 'late matching preserves the original proof identity');
      assert.equal(qualified.receipt_auth.requalificationLinks.length, 0);
      assert.equal((await transfer('10-direct')).completion_auth.requalificationLinks.length, 1,
        'the late counterpart binds its archived checkpoint without rewriting the earlier receipt authorization');
      assert.equal((await transfer('10-direct')).status, 'Completed');
      assert.deepEqual((await db.query('SELECT * FROM completed_transfer WHERE id<>$1 ORDER BY id', [freshId])).rows, legacyRows);
      assert.deepEqual(await transfer('20-retained'), legacyTransfer);
      assert.ok(!fixture.state.requests.some(({ method, params }) =>
        method === 'eth_getTransactionReceipt' && params[0] === absentHash),
      'repair-owned missing originals are excluded in SQL before the 1000-row live page');
      await insert('transfer', { ...base, id: '15-missing', status: 'Bridging', nonce: '1001',
        completed_at: null, completed_at_block: null, completed_at_tx_hash: null,
        legacy_resolution: { status: 'Completed', completedAt: time, completedAtBlock: '11', completedAtTxHash: absentHash } });
      await insert('completed_transfer', { id: `outbound:${identity.queue}:1001`, src_network: 'Vara', dest_network: 'Ethereum',
        tx_hash: absentHash, block_number: '11', timestamp: time, matched: false, legacy_matched: true,
        outbound_receipt: { ...outbound, nonce: '1001', receipt: { ...outbound.receipt, transactionHash: absentHash } } });
      const committed = await snapshot();
      for (const args of [[], ['--apply']]) {
        const failure = await run('vara-to-eth', ['--after', '10-direct', '--limit', '1', ...args], false);
        assert.match(failure.stderr, /HOLD|unavailable|receipt/i);
        assert.ok(failure.stderr.includes(fixtureId('15-missing')), 'repair identifies the held original');
        assert.deepEqual(await snapshot(), committed, 'HOLD has no committed cursor or partial evidence');
      }
      const skipped = await run('vara-to-eth', ['--after', '20-retained', '--limit', '1']);
      assert.equal(skipped.lastExaminedId, '30-near'); assert.equal(skipped.qualified, 0);
      assert.deepEqual(await snapshot(), committed, 'an operator page skip does not certify the held row');
      const visible = (await graphQL(port)).get('15-missing');
      assert.equal(visible.status.toLowerCase(), 'bridging'); assert.equal(visible.completedAt, null);
      assert.equal(visible.completedAtBlock, null); assert.equal(visible.completedAtTxHash, null);

      // This scenario reuses a signed source receipt at a fresh settlement slot.
      // Park the separate repair fixture so it cannot claim that same original.
      parkedSettlement = (await db.query("DELETE FROM receipt_settlement WHERE id='200:2:0' RETURNING *")).rows[0];
      const deposit = { slot: '9000', transaction_index: '2', log_index: '0', deposit_count: '1',
        operation_id: hash('e1'), eth_token_id: identity.token, vara_token_id: identity.varaToken,
        sender: identity.receiver, receiver: identity.sender, amount: '10', native: true };
      await insert('transfer', { ...inbound, id: 'gear-missing', status: 'Bridging', tx_hash: absentHash,
        source_transaction_index: '99', source_log_index: '0',
        completed_at: null, completed_at_block: null, completed_at_tx_hash: null });
      await insert('transfer', { ...inbound, id: 'gear-fresh', status: 'Bridging', tx_hash: fixture.txHashes[2],
        source_transaction_index: '2', source_log_index: '0',
        completed_at: null, completed_at_block: null, completed_at_tx_hash: null });
      await db.query(`INSERT INTO receipt_relay (id, block_number)
        SELECT (1000+n)::text || ':99', 11 FROM generate_series(1,1001) n`);
      await db.query(`INSERT INTO receipt_settlement (id, deposit, timestamp, block_number, tx_hash, matched, legacy_matched)
        SELECT (1000+n)::text || ':99:0',
          jsonb_set(jsonb_set($1::jsonb, '{slot}', to_jsonb((1000+n)::text)), '{transaction_index}', '"99"'::jsonb),
          $2, 300, $3, false, n % 2 = 0 FROM generate_series(1,1001) n`, [deposit, time, hash('d0')]);
      const oldSettlements = (await db.query("SELECT * FROM receipt_settlement WHERE id LIKE '%:99:0' ORDER BY id")).rows;
      assert.equal(oldSettlements.length, 1001);
      const gear = new GearState();
      let gearHeight = 300;
      const liveGear = () => database.transaction('SERIALIZABLE', async (manager) => {
        await gear.new({ store: new Store(() => manager), log: quiet,
          blocks: [{ header: { height: gearHeight, hash: hash('db'), timestamp: 1_700_000_012_000 } }] });
        gear.recordReceiptRelay({ slot: 9000, transaction_index: 2, block_number: 11 });
        gear.recordReceiptSettlement(deposit, new Date(time), 300n, hash('d0'));
        await gear.save();
      });
      fixture.state.requests.length = 0;
      await liveGear();
      const freshGear = await transfer('gear-fresh');
      assert.equal(freshGear.status, 'Completed', 'Gear live page must reach fresh work beyond 1001 migrated settlements');
      assert.equal(freshGear.source_receipt_auth.version, 2);
      assert.deepEqual((await db.query("SELECT * FROM receipt_settlement WHERE id LIKE '%:99:0' ORDER BY id")).rows, oldSettlements);
      assert.equal((await transfer('gear-missing')).status, 'Bridging');
      assert.ok(!fixture.state.requests.some(({ method, params }) =>
        method === 'eth_getTransactionReceipt' && params[0] === absentHash));
      await db.query("UPDATE receipt_settlement SET matched=false, legacy_matched=true WHERE id='9000:2:0'");
      const corruptAuth = structuredClone(freshGear.source_receipt_auth);
      corruptAuth.receipt.logs[0].data = '0x';
      await db.query("UPDATE transfer SET source_receipt_auth=$1 WHERE id='gear-fresh'", [corruptAuth]);
      const beforeGearConflict = await snapshot();
      gearHeight++;
      await assert.rejects(liveGear(), /HOLD/);
      assert.deepEqual(await snapshot(), beforeGearConflict, 'authenticated conflicts remain atomic HOLDs');
      await db.query("UPDATE transfer SET source_receipt_auth=$1 WHERE id='gear-fresh'", [freshGear.source_receipt_auth]);
      await liveGear();
      assert.equal((await db.query("SELECT matched FROM receipt_settlement WHERE id='9000:2:0'")).rows[0].matched, true,
        'a migrated row with real source authorization is eligible again');
      assert.deepEqual((await transfer('gear-fresh')).source_receipt_auth, freshGear.source_receipt_auth);
    } finally {
      fixture.state.fault = undefined;
      destroyEthereumEvidenceClients('eth');
      destroyEthereumEvidenceClients('gear');
      await db.query("DELETE FROM receipt_settlement WHERE id LIKE '%:99:0' OR id='9000:2:0'");
      if (parkedSettlement) await insert('receipt_settlement', parkedSettlement);
      await db.query("DELETE FROM receipt_relay WHERE id LIKE '%:99' OR id='9000:2'");
      await db.query("DELETE FROM transfer WHERE id IN ('gear-missing','gear-fresh')");
      await db.query('DELETE FROM completed_transfer WHERE id IN ($1,$2) OR id LIKE $3',
        [freshId, `outbound:${identity.queue}:1001`, 'legacy-backlog-%']);
      await db.query('DELETE FROM transfer WHERE id=$1', [fixtureId('15-missing')]);
      await db.query(`UPDATE transfer SET nonce=$2, status=$3, completed_at=$4,
        completed_at_block=$5, completed_at_tx_hash=$6, completion_auth=$7 WHERE id=$1`,
      [lateTransfer.id, lateTransfer.nonce, lateTransfer.status, lateTransfer.completed_at,
        lateTransfer.completed_at_block, lateTransfer.completed_at_tx_hash, lateTransfer.completion_auth]);
      if (database.isInitialized) await database.destroy();
      for (const [key, value] of Object.entries(oldEnv)) {
        if (value === undefined) delete process.env[key]; else process.env[key] = value;
      }
    }
  });

  await t.test('base nonce-keyed completion repairs a coordinate-free late transfer and retains the old row', async () => {
    const pending = await transfer('00-pending');
    await db.query('UPDATE transfer SET nonce=$2 WHERE id=$1', [pending.id, '8']);
    await insert('completed_transfer', { id: '8', src_network: 'Vara', dest_network: 'Ethereum',
      tx_hash: fixture.txHashes[0], block_number: '11', timestamp: time, matched: false, legacy_matched: false });
    const old = (await db.query("SELECT * FROM completed_transfer WHERE id='8'")).rows[0];
    try {
      assert.equal((await run('vara-to-eth', ['--limit', '1'])).qualified, 1);
      assert.equal((await transfer('00-pending')).completion_auth, null, 'dry-run cannot authorize the hint');
      assert.equal((await run('vara-to-eth', ['--limit', '1', '--apply'])).qualified, 1);
      const repaired = await transfer('00-pending');
      assert.equal(repaired.status, 'Completed');
      assert.equal(repaired.completed_at_tx_hash, old.tx_hash);
      assert.equal(repaired.completion_auth.completionId, `outbound:${identity.queue}:8`);
      assert.deepEqual((await db.query("SELECT * FROM completed_transfer WHERE id='8'")).rows[0], old);
    } finally {
      await db.query('DELETE FROM completed_transfer WHERE id IN ($1,$2)', ['8', `outbound:${identity.queue}:8`]);
      await db.query(`UPDATE transfer SET nonce=$2, status=$3, completed_at=$4,
        completed_at_block=$5, completed_at_tx_hash=$6, completion_auth=$7 WHERE id=$1`,
      [pending.id, pending.nonce, pending.status, pending.completed_at,
        pending.completed_at_block, pending.completed_at_tx_hash, pending.completion_auth]);
    }
  });

  await t.test('dry-run, malformed arguments and missing configuration never write, even checkpoints', async () => {
    const saved = await snapshot();
    await run('vara-to-eth');
    const inbound = await run('eth-to-vara');
    assert.equal(inbound.qualified, 2, 'base-era NULL coordinates qualify against the original receipts without writes');
    for (const id of ['40-inbound', '50-refund']) {
      assert.equal((await transfer(id)).source_transaction_index, null);
      assert.equal((await transfer(id)).source_log_index, null);
    }
    for (const args of [['--limit', '0'], ['--limit', '1001'], ['--unknown'], ['--after', '']]) await run('vara-to-eth', args, false);
    await run('invalid', [], false);
    const missing = { ...env, ETH_FINALITY_BOOTSTRAP: '' }; // Do not let a local .env fill deliberately missing authority.
    const result = await launch('lib/eth/reconcile.js', ['--direction', 'vara-to-eth', '--apply'], missing).done;
    assert.notEqual(result.code, 0);
    assert.deepEqual(await snapshot(), saved);
  });

  await t.test('one-record page advances a missing counterpart but never certifies it', async () => {
    const report = await run('vara-to-eth', ['--limit', '1', '--apply']);
    assert.equal(report.lastExaminedId, '00-pending'); assert.equal(report.hasMore, true);
    assert.equal(report.qualified, 0); assert.equal(report.pending, 1);
    const row = await transfer('00-pending'); assert.equal(row.completion_auth, null); assert.equal(row.status, 'Bridging');
    assert.equal((await transfer('10-direct')).status, 'Bridging');
  });

  await t.test('a corrupt receipt trie after the first candidate rolls the entire page back', async () => {
    const saved = await snapshot();
    let receiptReads = 0;
    fixture.state.fault = ({ method, result }) => {
      if (method === 'eth_getBlockReceipts' && ++receiptReads === 2) return result.map((receipt, index) =>
        index === 1 ? { ...receipt, status: '0x0' } : receipt);
      return result;
    };
    await run('vara-to-eth', ['--after', '00-pending', '--limit', '2', '--apply'], false, true);
    assert.equal(receiptReads, 2, 'the second original must fail after the first native proof succeeded');
    fixture.state.fault = undefined;
    assert.deepEqual(await snapshot(), saved);
  });

  await t.test('late inbound disagreement rolls back staged base-era coordinate recovery', async () => {
    const saved = await snapshot();
    let receiptReads = 0;
    fixture.state.fault = ({ method, result }) => method === 'eth_getBlockReceipts' && ++receiptReads === 2
      ? result.map((receipt, index) => index === 3 ? { ...receipt, status: '0x0' } : receipt) : result;
    try {
      await run('eth-to-vara', ['--limit', '2', '--apply'], false, true);
      assert.equal(receiptReads, 2, 'failure must follow the first staged source-coordinate recovery');
      assert.deepEqual(await snapshot(), saved);
    } finally { fixture.state.fault = undefined; }
  });

  await t.test('apply restores only original effects, preserves legacy JSON and distinguishes native refund', async () => {
    const first = await run('vara-to-eth', ['--after', '00-pending', '--limit', '1', '--apply']);
    assert.equal(first.lastExaminedId, '10-direct'); assert.equal(first.qualified, 1); assert.equal(first.hasMore, true);
    assert.equal((await transfer('20-retained')).status, 'Bridging');
    const next = await run('vara-to-eth', ['--after', first.lastExaminedId, '--limit', '1', '--apply']);
    assert.equal(next.lastExaminedId, '20-retained'); assert.equal(next.qualified, 1);
    const near = await run('vara-to-eth', ['--after', next.lastExaminedId, '--limit', '1', '--apply']);
    assert.equal(near.lastExaminedId, '30-near'); assert.equal(near.qualified, 0); assert.equal(near.pending, 1);
    const incoming = await run('eth-to-vara', ['--limit', '1', '--apply']);
    assert.equal(incoming.lastExaminedId, '40-inbound'); assert.equal(incoming.qualified, 1); assert.equal(incoming.hasMore, true);
    assert.equal((await transfer('50-refund')).status, 'Bridging');
    const refund = await run('eth-to-vara', ['--after', incoming.lastExaminedId, '--limit', '1', '--apply']);
    assert.equal(refund.lastExaminedId, '50-refund'); assert.equal(refund.qualified, 1); assert.equal(refund.hasMore, false);
    for (const id of ['10-direct', '20-retained', '40-inbound']) assert.equal((await transfer(id)).status, 'Completed');
    assert.equal((await transfer('50-refund')).status, 'Refunded');
    const direct = await transfer('10-direct');
    assert.equal(direct.completed_at.toISOString(), new Date(1_700_000_011_000).toISOString(), 'EL terminal time comes from qualified header');
    assert.equal(direct.completion_auth.completionId, `outbound:${identity.queue}:8`);
    assert.equal((await transfer('40-inbound')).source_receipt_auth.localLogIndex, 0, 'receipt-local index is not the block-global index');
    assert.equal((await transfer('50-refund')).completion_auth.outcome, 'Refunded');
    for (const [id, index] of [['40-inbound', '2'], ['50-refund', '3']]) {
      const row = await transfer(id);
      assert.equal(row.source_transaction_index, index);
      assert.equal(row.source_log_index, '0', 'backfill the receipt-relative index, never the block-global index');
    }
    assert.deepEqual((await db.query('SELECT id, deposit FROM receipt_settlement ORDER BY id')).rows, depositsBefore);
    assert.deepEqual((await db.query('SELECT outbound_receipt FROM completed_transfer WHERE id = $1', [`outbound:${identity.queue}:9`])).rows[0].outbound_receipt, outbound);
    assert.equal((await transfer('30-near')).status, 'Bridging');
    assert.equal((await transfer('30-near')).completion_auth, null);
    const checkpoints = (await db.query('SELECT * FROM ethereum_finality_checkpoint ORDER BY block_number')).rows;
    assert.deepEqual(checkpoints.find((row) => row.id === legacyCheckpointId).checkpoint, legacyCheckpoint);
    const nativeRows = checkpoints.filter((row) => row.checkpoint.schemaVersion === 2);
    assert.ok(nativeRows.length > 0);
    for (const row of nativeRows) {
      assert.equal(row.block_number, '11');
      assert.equal(row.checkpoint.policy, 'beacon-sync-committee-v1');
      assert.ok(row.checkpoint.native, 'retain exact native Beacon proof metadata, not just a flat header');
      assert.ok(row.id.includes('beacon-sync-committee-v1'), 'new checkpoint identity is policy-qualified');
    }
    for (const label of ['10-direct', '20-retained', '40-inbound', '50-refund']) {
      const row = await transfer(label);
      const archived = JSON.parse(nativeMigrated.transfer).find((original) => original.id === row.id);
      assert.deepEqual(row.legacy_ethereum_auth, archived.legacy_ethereum_auth, 'repair cannot rewrite historical authorization');
      assert.deepEqual(row.legacy_resolution, archived.legacy_resolution, 'repair cannot rewrite the prior terminal projection');
      for (const auth of [row.source_receipt_auth, row.completion_auth].filter(Boolean)) {
        assert.equal(auth.version, 2);
        assert.match(auth.inclusionProofDigest, /^0x[0-9a-f]{64}$/);
        assert.match(auth.checkpointProofDigest, /^0x[0-9a-f]{64}$/);
        assert.ok(auth.requalificationLinks.length > 0, 'old checkpoint authority requires a separate native proof link');
        for (const link of auth.requalificationLinks) {
          assert.match(link.originalDigest, /^0x[0-9a-f]{64}$/); assert.match(link.proofDigest, /^0x[0-9a-f]{64}$/);
        }
      }
    }
    assert.deepEqual((await db.query('SELECT legacy_receipt_auth FROM completed_transfer WHERE id=$1',
      [`outbound:${identity.queue}:9`])).rows[0].legacy_receipt_auth,
    JSON.parse(nativeMigrated.completed_transfer).find((row) => row.id === `outbound:${identity.queue}:9`).legacy_receipt_auth);
    const visible = await graphQL(port);
    for (const id of ['10-direct', '20-retained', '40-inbound', '50-refund']) {
      assert.ok(visible.get(id).completedAt); assert.notEqual(visible.get(id).status.toLowerCase(), 'bridging');
    }
    assert.equal(visible.get('50-refund').status.toLowerCase(), 'refunded');
    assert.equal(visible.get('30-near').completedAt, null);
    console.log(JSON.stringify({ evidenceDbObserved: { qualified: 4, pendingNearMatch: 1, refunded: 1, checkpointHeight: 11 } }));
  });

  await t.test('new CLI processes revalidate terminal rows idempotently without rebinding advancing finality', async () => {
    const saved = await snapshot();
    await run('vara-to-eth', ['--apply']); await run('eth-to-vara', ['--apply']);
    assert.deepEqual(await snapshot(), saved, 'duplicate restart must not rewrite original evidence');
    fixture.state.finalizedHeight = 12;
    await run('vara-to-eth', ['--apply']); await run('eth-to-vara', ['--apply']);
    const advanced = await snapshot();
    for (const table of tables.filter((name) => name !== 'ethereum_finality_checkpoint')) assert.equal(advanced[table], saved[table], table);
    assert.equal((await db.query('SELECT max(block_number)::text AS height FROM ethereum_finality_checkpoint')).rows[0].height, '12');
  });

  await t.test('Beacon outage, another network and corrupted original proofs preserve terminal evidence', async () => {
    const saved = await snapshot();
    const faults = [
      ({ side, result }) => { if (side === 'beacon') throw new Error('fixture Beacon unavailable'); return result; },
      ({ method, result }) => method === 'eth_chainId' ? '0x1' : result,
      ({ method, result }) => method === 'eth_getBlockReceipts' && result
        ? result.map((receipt, index) => index === 0 ? { ...receipt, status: '0x0' } : receipt) : result,
      ({ method, result }) => ['eth_getBlockByNumber', 'eth_getBlockByHash'].includes(method) && result
        ? { ...result, hash: hash('ff') } : result,
    ];
    for (const fault of faults) {
      fixture.state.fault = fault;
      await run('vara-to-eth', ['--after', '00-pending', '--limit', '1', '--apply'], false, true);
      assert.deepEqual(await snapshot(), saved);
    }
    fixture.state.fault = ({ method, result }) => method === 'eth_getBlockReceipts' && result
      ? result.map((receipt, index) => index === 2
        ? { ...receipt, logs: receipt.logs.map((log) => ({ ...log, data: `0x${'00'.repeat(64)}` })) } : receipt) : result;
    await run('eth-to-vara', ['--apply'], false, true);
    fixture.state.fault = undefined;
    assert.deepEqual(await snapshot(), saved);
  });

  await t.test('persisted requalification proof links cannot be replaced during dry-run or apply', async () => {
    for (const [table, column, id, direction] of [
      ['transfer', 'completion_auth', fixtureId('10-direct'), 'vara-to-eth'],
      ['transfer', 'source_receipt_auth', fixtureId('40-inbound'), 'eth-to-vara'],
      ['transfer', 'completion_auth', fixtureId('40-inbound'), 'eth-to-vara'],
      ['completed_transfer', 'receipt_auth', `outbound:${identity.queue}:9`, 'vara-to-eth'],
    ]) {
      const original = (await db.query(`SELECT "${column}" AS auth FROM "${table}" WHERE id=$1`, [id])).rows[0].auth;
      assert.equal(original.version, 2);
      assert.ok(original.requalificationLinks.length > 0, 'the regression must exercise a real native legacy requalification link');
      assert.notEqual(original.requalificationLinks[0].proofDigest, hash('ff'));
      const before = await snapshot();
      try {
        await db.query(`UPDATE "${table}" SET "${column}" =
          jsonb_set("${column}", '{requalificationLinks,0,proofDigest}', to_jsonb($2::text)) WHERE id=$1`, [id, hash('ff')]);
        const corrupted = await snapshot();
        for (const args of [[], ['--apply']]) {
          const failure = await run(direction, args, false);
          assert.match(failure.stderr, /HOLD/i);
          assert.deepEqual(await snapshot(), corrupted, 'an invalid retained link cannot produce writes or a success cursor');
        }
      } finally {
        await db.query(`UPDATE "${table}" SET "${column}"=$2 WHERE id=$1`, [id, original]);
      }
      assert.deepEqual(await snapshot(), before, 'restore only the deliberately corrupted authorization');
    }
  });

  await t.test('changed terminal IDs, outcomes and coordinates HOLD rather than replace original bindings', async () => {
    const cases = [
      ['10-direct', 'vara-to-eth', "completion_auth = jsonb_set(completion_auth, '{completionId}', '\"wrong-original\"')"],
      ['10-direct', 'vara-to-eth', 'completed_at_block = 999'],
      ['40-inbound', 'eth-to-vara', "completion_auth = jsonb_set(completion_auth, '{receiptSettlementId}', '\"wrong-original\"')"],
      ['50-refund', 'eth-to-vara', "completion_auth = jsonb_set(completion_auth, '{outcome}', '\"Completed\"')"],
      ['50-refund', 'eth-to-vara', "status = 'Completed'"],
      ['40-inbound', 'eth-to-vara', "completed_at_tx_hash = 'wrong-original'"],
    ];
    for (const [id, direction, mutation] of cases) {
      const original = await transfer(id);
      await db.query(`UPDATE transfer SET ${mutation} WHERE id = $1`, [fixtureId(id)]);
      const saved = await snapshot();
      await run(direction, ['--apply'], false); assert.deepEqual(await snapshot(), saved);
      await db.query('UPDATE transfer SET status=$2, completion_auth=$3, completed_at_block=$4, completed_at_tx_hash=$5 WHERE id=$1',
        [fixtureId(id), original.status, original.completion_auth, original.completed_at_block, original.completed_at_tx_hash]);
    }
  });

  await t.test('a concurrent live write aborts the stale SERIALIZABLE repair page', async () => {
    let committed;
    let raced = false;
    fixture.state.fault = async ({ side, method, result }) => {
      if (!raced && side === 'primary' && method === 'eth_getBlockReceipts') {
        raced = true;
        // The CLI has selected its transfer snapshot but has not saved its staged changes.
        await db.query('UPDATE transfer SET amount = $2 WHERE id = $1', [fixtureId('10-direct'), '11']);
        committed = await snapshot();
      }
      return result;
    };
    try {
      const failure = await run('vara-to-eth', ['--after', '00-pending', '--limit', '1', '--apply'], false, true);
      assert.equal(raced, true, 'the race must reach the selected original receipt');
      assert.match(failure.stderr, /serializ|concurrent update|40001/i, 'must abort the stale transaction, not overwrite the live writer');
      assert.deepEqual(await snapshot(), committed);
    } finally {
      fixture.state.fault = undefined;
      await db.query('UPDATE transfer SET amount = $2 WHERE id = $1', [fixtureId('10-direct'), '10']);
    }
  });

  await t.test('real checkpoint insert races retry the whole transaction and never overwrite a winner', async () => {
    await import('reflect-metadata');
    const { DataSource } = await import('typeorm');
    const { Store } = await import('@subsquid/typeorm-store');
    const { EthereumFinalityCheckpoint } = await import('../lib/model/entities/ethereumFinalityCheckpoint.model.js');
    const { retainCheckpoint, checkpointId, destroyEthereumEvidenceClients } = await import('../lib/eth/finality.js');
    const database = new DataSource({ type: 'postgres', url: url.href, entities: [EthereumFinalityCheckpoint],
      synchronize: false, migrationsRun: false });
    await database.initialize();
    const saved = await snapshot();
    const next = await fixture.checkpoint(13, 'eth');
    destroyEthereumEvidenceClients('eth');
    fixture.state.finalizedHeight = 12;
    const id = checkpointId(next);
    try {
      for (const conflicting of [false, true]) {
        const winner = conflicting ? { ...next, stateRoot: hash('ff') } : next;
        const first = database.createQueryRunner(), second = database.createQueryRunner();
        try {
          await first.connect(); await second.connect();
          await first.startTransaction('SERIALIZABLE'); await second.startTransaction('SERIALIZABLE');
          const firstStore = new Store(() => first.manager), secondStore = new Store(() => second.manager);
          const find = firstStore.findOneBy.bind(firstStore);
          firstStore.findOneBy = async (...args) => {
            const result = await find(...args);
            assert.ok(result == null);
            if (conflicting) await second.manager.query(
              'INSERT INTO ethereum_finality_checkpoint (id, block_number, checkpoint) VALUES ($1,$2,$3)', [id, next.number, winner]);
            else await retainCheckpoint(secondStore, winner);
            await second.commitTransaction();
            return result;
          };
          await assert.rejects(retainCheckpoint(firstStore, next), (error) => {
            assert.equal(error.code, '40001', 'race must surface as a whole-transaction retry, not continue aborted SQL');
            return true;
          });
          await first.rollbackTransaction();
          await first.startTransaction('SERIALIZABLE');
          const retry = new Store(() => first.manager);
          if (conflicting) await assert.rejects(retainCheckpoint(retry, next), /conflicting retained Ethereum checkpoint|native checkpoint projection mismatch/);
          else await retainCheckpoint(retry, next);
          await first.rollbackTransaction();
          assert.deepEqual((await db.query('SELECT checkpoint FROM ethereum_finality_checkpoint WHERE id = $1', [id])).rows[0].checkpoint, winner);
        } finally {
          if (first.isTransactionActive) await first.rollbackTransaction();
          if (second.isTransactionActive) await second.rollbackTransaction();
          await first.release(); await second.release();
          await db.query('DELETE FROM ethereum_finality_checkpoint WHERE id = $1', [id]);
        }
      }
      assert.deepEqual(await snapshot(), saved);
    } finally { await database.destroy(); }
  });

  await t.test('a retained foreign-network checkpoint blocks restart even below the highest local checkpoint', async () => {
    const foreign = { ...legacyCheckpoint, chainId: '1', number: '1', hash: fixture.headers[1].hash,
      parentHash: fixture.headers[0].hash, timestamp: '1700000001' };
    const id = `1:${foreign.genesisHash}:1`;
    await db.query('INSERT INTO ethereum_finality_checkpoint (id, block_number, checkpoint) VALUES ($1, 1, $2)', [id, foreign]);
    try {
      const saved = await snapshot();
      await run('vara-to-eth', ['--apply'], false);
      assert.deepEqual(await snapshot(), saved);
    } finally { await db.query('DELETE FROM ethereum_finality_checkpoint WHERE id = $1', [id]); }
  });

  await t.test('HTML7fe1b032 reconstructed: rollback refuses retained unmapped liabilities', async () => {
    await insert('transfer', { ...inbound, id: 'unmapped-liability', destination: null,
      status: 'Bridging', completed_at: null, completed_at_block: null, completed_at_tx_hash: null });
    try {
      const saved = await snapshot();
      await assert.rejects(migrate('down'), /Cannot.*unmapped deposit liabilities/i);
      assert.deepEqual(await snapshot(), saved);
    } finally { await db.query("DELETE FROM transfer WHERE id='unmapped-liability'"); }
  });
});
