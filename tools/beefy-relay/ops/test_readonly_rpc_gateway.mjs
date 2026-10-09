import assert from 'node:assert/strict';
import { once } from 'node:events';
import http from 'node:http';
import WebSocket, { WebSocketServer } from 'ws';
import { createGateway } from './readonly-rpc-gateway.mjs';

// This upstream is disposable: no test connects to or writes to an owned node.
const token = 'ab'.repeat(32);
const hash = '0x' + '11'.repeat(32);
const backend = http.createServer();
const upstream = new WebSocketServer({ server: backend });
const received = [];
const clients = new Set();
let connections = 0;
let subscription = 0;
let gateway;
const guard = setTimeout(() => { throw new Error('Gateway boundary tests timed out'); }, 15_000);

function bounded(promise, label) {
  let timer;
  return Promise.race([promise, new Promise((_, reject) => {
    timer = setTimeout(() => reject(new Error(label + ' timed out')), 2000);
  })]).finally(() => clearTimeout(timer));
}
async function listen(server) {
  server.listen(0, '127.0.0.1');
  await once(server, 'listening');
  return server.address().port;
}
function inbox(socket) {
  const queue = [];
  let pending;
  socket.on('message', data => {
    const value = JSON.parse(data.toString());
    if (pending) { const resolve = pending; pending = undefined; resolve(value); }
    else queue.push(value);
  });
  return () => queue.length ? Promise.resolve(queue.shift()) : bounded(new Promise(resolve => {
    assert.equal(pending, undefined, 'Only one pending read per socket');
    pending = resolve;
  }), 'RPC response');
}
async function connect(url) {
  const socket = new WebSocket(url);
  clients.add(socket);
  socket.on('error', () => {});
  const next = inbox(socket);
  await bounded(once(socket, 'open'), 'Client connection');
  return { socket, next };
}
async function close(socket) {
  if (socket.readyState === WebSocket.CLOSED) return;
  const closed = once(socket, 'close');
  socket.close();
  await bounded(closed, 'Client close');
}
async function unauthorized(url) {
  const socket = new WebSocket(url);
  clients.add(socket);
  socket.on('error', () => {});
  const status = await bounded(new Promise((resolve, reject) => {
    socket.once('open', () => reject(new Error('Unauthorized socket opened')));
    socket.once('unexpected-response', (_, response) => {
      response.resume();
      resolve(response.statusCode);
      socket.terminate();
    });
  }), 'Unauthorized handshake');
  assert.equal(status, 401);
  assert.equal(connections, 0, 'Unauthorized handshake connected upstream');
}
async function httpStatus(port, path) {
  return bounded(new Promise((resolve, reject) => {
    http.get({ hostname: '127.0.0.1', port, path }, response => {
      response.resume();
      resolve(response.statusCode);
    }).on('error', reject);
  }), 'HTTP request');
}

upstream.on('connection', socket => {
  connections++;
  socket.on('error', () => {});
  socket.on('message', data => {
    const request = JSON.parse(data.toString());
    received.push(request);
    if (request.method === 'rpc_methods') {
      socket.send(JSON.stringify({ jsonrpc: '2.0', id: request.id, result: { version: 1,
        methods: ['chain_getHeader', 'state_call', 'author_submitExtrinsic', 'system_addReservedPeer', 'unknown_rpc'] } }));
    } else if (request.method.includes('_subscribe')) {
      const id = 'subscription-' + ++subscription;
      socket.send(JSON.stringify({ jsonrpc: '2.0', id: request.id, result: id }));
      const method = { chain_subscribeFinalizedHeads: 'chain_finalizedHead',
        state_subscribeRuntimeVersion: 'state_runtimeVersion', state_subscribeStorage: 'state_storage' }[request.method];
      socket.send(JSON.stringify({ jsonrpc: '2.0', method, params: { subscription: id, result: { height: 42 } } }));
    } else {
      socket.send(JSON.stringify({ jsonrpc: '2.0', id: request.id,
        result: request.method.includes('_unsubscribe') ? true : { method: request.method, params: request.params ?? [] } }));
    }
  });
});

try {
  const backendPort = await listen(backend);
  gateway = createGateway({ upstream: `ws://127.0.0.1:${backendPort}`, token });
  assert.ok(gateway instanceof http.Server);
  const port = await listen(gateway);
  const base = `ws://127.0.0.1:${port}`;
  const url = base + '/rpc/' + token;
  for (const path of ['/', '/rpc/', '/rpc/' + '00'.repeat(32), '/rpc/' + token.toUpperCase(),
    '/rpc/' + token.slice(1), '/rpc/' + token + '/', '/rpc/' + token + '?token=' + token,
    '/rpc/%61' + token.slice(1)]) await unauthorized(base + path);
  assert.equal(await httpStatus(port, '/rpc/' + token), 404, 'Non-upgrade HTTP is not an RPC transport');
  assert.equal(connections, 0);

  const client = await connect(url);
  let id = 0;
  async function request(method, params = [], requestId = ++id) {
    const frame = { jsonrpc: '2.0', id: requestId, method, params };
    const before = received.length;
    client.socket.send(JSON.stringify(frame));
    const response = await client.next();
    assert.equal(response.id, requestId, 'Request ID changed');
    assert.equal(response.jsonrpc, '2.0');
    assert.equal(response.error, undefined, method + ' unexpectedly rejected');
    assert.equal(received.length, before + 1, method + ' was not forwarded exactly once');
    assert.deepEqual(received[before], { ...frame, id: received[before].id }, 'Forwarded method or params changed');
    return response.result;
  }

  const allowed = [
    ['system_chain', []], ['system_properties', []], ['system_health', []],
    ['chain_getBlockHash', [0]], ['chain_getBlockHash', ['0x2a']],
    ['chain_getHeader', []], ['chain_getHeader', [hash]], ['chain_getFinalizedHead', []],
    ['state_getRuntimeVersion', []], ['state_getRuntimeVersion', [hash]], ['state_getMetadata', []],
    ['state_getMetadata', [hash]], ['state_getStorage', ['0x3a636f6465', hash]],
    ['state_queryStorageAt', [['0x0102'], hash]],
    ['state_call', ['Metadata_metadata_versions', '0x']],
    ['state_call', ['Metadata_metadata_at_version', '0x0f000000', hash]],
    ['state_call', ['Metadata_metadata', '0x', hash]],
    ['gear_calculateReplyForHandle', [hash, hash, '0x00ff', '1000000', '0', hash]],
    ['gear_calculateReplyForHandle', [hash, hash, '00ff', 1000000, 0, null]],
    ['gearEthBridge_merkleProof', [hash, hash]],
  ];
  for (const [method, params] of allowed) {
    assert.deepEqual(await request(method, params), { method, params });
  }
  assert.deepEqual(await request('chain_getHeader', [hash], 'caller-string-id'), { method: 'chain_getHeader', params: [hash] });
  assert.deepEqual(await request('chain_getHeader', [hash], 0), { method: 'chain_getHeader', params: [hash] });
  assert.deepEqual(await request('rpc_methods'), { version: 1, methods: ['chain_getHeader', 'state_call'] });
  const omittedId = ++id;
  client.socket.send(JSON.stringify({ jsonrpc: '2.0', id: omittedId, method: 'system_health' }));
  assert.deepEqual(await client.next(), { jsonrpc: '2.0', id: omittedId, result: { method: 'system_health', params: [] } });

  for (const [subscribe, unsubscribe] of [
    ['chain_subscribeFinalizedHeads', 'chain_unsubscribeFinalizedHeads'],
    ['state_subscribeRuntimeVersion', 'state_unsubscribeRuntimeVersion'],
    ['state_subscribeStorage', 'state_unsubscribeStorage'],
  ]) {
    const subId = await request(subscribe, subscribe === 'state_subscribeStorage' ? [['0x0102']] : []);
    const notification = await client.next();
    assert.deepEqual(notification, { jsonrpc: '2.0', method: {
      chain_subscribeFinalizedHeads: 'chain_finalizedHead', state_subscribeRuntimeVersion: 'state_runtimeVersion',
      state_subscribeStorage: 'state_storage',
    }[subscribe], params: { subscription: subId, result: { height: 42 } } });
    assert.equal(await request(unsubscribe, [subId]), true);
  }

  async function reject(raw, expectedId) {
    const before = received.length;
    client.socket.send(typeof raw === 'string' ? raw : JSON.stringify(raw));
    const response = await client.next();
    assert.ok(response.error, 'Invalid request was not rejected');
    if (expectedId !== undefined) assert.equal(response.id, expectedId);
    assert.equal(received.length, before, 'Rejected request reached upstream');
  }
  const denied = [
    ['author_submitExtrinsic', ['0x00']], ['author_submitAndWatchExtrinsic', ['0x00']],
    ['author_insertKey', ['babe', '//Alice', hash]], ['author_rotateKeys', []],
    ['author_removeExtrinsic', [[hash]]], ['author_unwatchExtrinsic', ['subscription-1']],
    ['system_addReservedPeer', ['/ip4/127.0.0.1/tcp/1234']], ['system_removeReservedPeer', ['peer']],
    ['dev_setStorage', [[['0x01', '0x02']]]], ['offchain_localStorageSet', ['PERSISTENT', '0x00', '0x01']],
    ['engine_createBlock', [true, true, null]], ['chainHead_v1_call', ['id', hash, 'Core_execute_block', '0x']],
    ['unknown_rpc', []], ['constructor', []], ['__proto__', []], ['Chain_getHeader', []],
    ['state_call', ['Core_execute_block', '0x00']], ['state_call', ['BlockBuilder_apply_extrinsic', '0x00']],
    ['state_call', ['TaggedTransactionQueue_validate_transaction', '0x00']], ['state_call', ['Core_version', '0x']],
    ['state_call', ['Metadata_metadata_versions', '0x00']], ['state_call', ['Metadata_metadata', '0x00']],
    ['state_call', ['Metadata_metadata_at_version', '0x0f00000000']],
    ['state_call', ['Metadata_metadata_at_version', '0x0f0000']],
    ['state_call', ['Metadata_metadata_versions', '0x', hash, 'extra']],
    ['state_call', ['Metadata_metadata_versions\u0000Core_execute_block', '0x']],
    ['chain_getHeader', ['http://evil.invalid']], ['chain_getHeader', [hash, hash]],
    ['chain_getBlockHash', [-1]], ['state_getStorage', ['not-hex', hash]],
    ['gear_calculateReplyForHandle', [hash, hash, '0x0', 1, 0, hash]],
    ['gear_calculateReplyForHandle', [hash, hash, '0x00', -1, 0, hash]],
    ['gear_calculateReplyForHandle', [hash, hash, '0x00', 1, -1, hash]],
    ['state_unsubscribeStorage', ['foreign-subscription']],
  ];
  for (const [method, params] of denied) {
    const requestId = ++id;
    await reject({ jsonrpc: '2.0', id: requestId, method, params }, requestId);
  }
  const valid = { jsonrpc: '2.0', id: ++id, method: 'chain_getHeader', params: [] };
  for (const malformed of ['{', 'null', '42', '"string"', '{}',
    { ...valid, jsonrpc: '1.0' }, { ...valid, method: 1 }, { ...valid, params: {} },
    { ...valid, id: {} }, { ...valid, id: null }, { ...valid, id: 1.5 },
    { jsonrpc: '2.0', method: 'chain_getHeader', params: [] },
    [], [valid], [valid, { ...valid, method: 'author_submitExtrinsic', params: ['0x00'] }], [[valid]],
  ]) await reject(malformed);
  // A round-trip barrier ensures none of the preceding rejected frames was queued upstream.
  const rejectedCount = received.length;
  await request('chain_getHeader');
  assert.equal(received.length, rejectedCount + 1);

  const beforeBinary = received.length;
  client.socket.send(Buffer.from(JSON.stringify(valid)), { binary: true });
  assert.ok((await client.next()).error, 'Binary JSON bypassed request validation');
  await request('chain_getHeader');
  assert.equal(received.length, beforeBinary + 1, 'Binary JSON reached upstream');
  const oversized = await connect(url);
  oversized.socket.send(JSON.stringify({ ...valid, id: ++id }));
  await oversized.next();
  const oversizedCount = received.length;
  const oversizedClosed = once(oversized.socket, 'close');
  const oversizedBackend = [...upstream.clients].find(socket => socket !== [...upstream.clients][0]);
  assert.ok(oversizedBackend);
  const oversizedBackendClosed = once(oversizedBackend, 'close');
  oversized.socket.send(' '.repeat(1024 * 1024 + 1));
  assert.ok([1006, 1009].includes((await bounded(oversizedClosed, 'Oversized frame rejection'))[0]));
  await bounded(oversizedBackendClosed, 'Oversized frame upstream cleanup');
  assert.equal(received.length, oversizedCount, 'Oversized frame reached upstream');

  const upstreamSocket = [...upstream.clients][0];
  assert.ok(upstreamSocket);
  const backendClosed = once(upstreamSocket, 'close');
  await close(client.socket);
  await bounded(backendClosed, 'Upstream cleanup on client disconnect');
  assert.equal(upstream.clients.size, 0);

  const remoteDisconnect = await connect(url);
  remoteDisconnect.socket.send(JSON.stringify({ ...valid, id: ++id }));
  await remoteDisconnect.next();
  const remoteClosed = once(remoteDisconnect.socket, 'close');
  [...upstream.clients][0].close();
  await bounded(remoteClosed, 'Client cleanup on upstream disconnect');
  assert.equal(upstream.clients.size, 0);

  const active = await connect(url);
  active.socket.send(JSON.stringify({ ...valid, id: ++id }));
  await active.next();
  const activeClientClosed = once(active.socket, 'close');
  const activeBackendClosed = once([...upstream.clients][0], 'close');
  await bounded(new Promise((resolve, reject) => gateway.close(error => error ? reject(error) : resolve())), 'Gateway shutdown');
  await Promise.all([bounded(activeClientClosed, 'Shutdown client cleanup'), bounded(activeBackendClosed, 'Shutdown upstream cleanup')]);
  assert.equal(upstream.clients.size, 0);
  console.log('Readonly RPC gateway boundary tests passed');
} finally {
  clearTimeout(guard);
  for (const socket of clients) socket.terminate();
  for (const socket of upstream.clients) socket.terminate();
  if (gateway?.listening) await new Promise(resolve => gateway.close(resolve));
  await new Promise(resolve => upstream.close(resolve));
  if (backend.listening) await new Promise(resolve => backend.close(resolve));
}
