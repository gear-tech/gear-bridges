import { timingSafeEqual } from 'node:crypto';
import { constants, openSync, fstatSync, readFileSync, closeSync } from 'node:fs';
import { createServer } from 'node:http';
import { pathToFileURL } from 'node:url';
import { parseArgs } from 'node:util';
import WebSocket, { WebSocketServer } from 'ws';

const REQUEST_BYTES = 1024 * 1024;
const RESPONSE_BYTES = 16 * REQUEST_BYTES;
const MAX_CLIENTS = 16;
const MAX_INFLIGHT = 32;
const MAX_SUBSCRIPTIONS = 16;
const TIMEOUT_MS = 30_000;
const hash = (value) => typeof value === 'string' && /^0x[0-9a-fA-F]{64}$/.test(value);
const bytes = (value) => typeof value === 'string' && /^0x(?:[0-9a-fA-F]{2})*$/.test(value);
const payload = (value) => typeof value === 'string' && /^(?:0x)?(?:[0-9a-fA-F]{2})*$/.test(value);
const optionalHash = (value) => value == null || hash(value);
const idValid = (value) => (typeof value === 'string' && value.length <= 128) || Number.isSafeInteger(value);
const object = (value) => value !== null && typeof value === 'object' && !Array.isArray(value);
const uint = (value, bits) => {
  if (typeof value === 'number') return Number.isSafeInteger(value) && value >= 0 && BigInt(value) < (1n << BigInt(bits));
  return typeof value === 'string' && value.length <= 40 && /^(?:[0-9]+|0x[0-9a-fA-F]+)$/.test(value)
    && BigInt(value) < (1n << BigInt(bits));
};
const noParams = (params) => params.length === 0;
const at = (params) => params.length <= 1 && optionalHash(params[0]);
const keysAt = (params) => params.length >= 1 && params.length <= 2 && Array.isArray(params[0])
  && params[0].length > 0 && params[0].length <= 256 && params[0].every(bytes) && optionalHash(params[1]);

// No prefix matching: simulation and metadata are reads; submission/admin/runtime execution are not.
const reads = new Map([
  ...['rpc_methods', 'system_chain', 'system_properties', 'system_health',
    'chain_getFinalizedHead'].map((method) => [method, noParams]),
  ...['chain_getHeader', 'state_getMetadata', 'state_getRuntimeVersion'].map((method) => [method, at]),
  ['chain_getBlockHash', (params) => params.length <= 1 && (params[0] == null || uint(params[0], 32))],
  ['state_getStorage', (params) => params.length >= 1 && params.length <= 2 && bytes(params[0]) && optionalHash(params[1])],
  ['state_queryStorageAt', keysAt],
  ['state_call', (params) => params.length >= 2 && params.length <= 3 && optionalHash(params[2]) && (
    (['Metadata_metadata', 'Metadata_metadata_versions'].includes(params[0]) && params[1] === '0x')
    || (params[0] === 'Metadata_metadata_at_version' && typeof params[1] === 'string' && /^0x[0-9a-fA-F]{8}$/.test(params[1]))
  )],
  ['gear_calculateReplyForHandle', (params) => params.length >= 5 && params.length <= 6
    && hash(params[0]) && hash(params[1]) && payload(params[2]) && uint(params[3], 64) && uint(params[4], 128) && optionalHash(params[5])],
  ['gearEthBridge_merkleProof', (params) => params.length >= 1 && params.length <= 2 && hash(params[0]) && optionalHash(params[1])],
]);
const subscriptions = new Map([
  ['chain_subscribeFinalizedHeads', ['chain_unsubscribeFinalizedHeads', 'chain_finalizedHead', noParams]],
  ['state_subscribeRuntimeVersion', ['state_unsubscribeRuntimeVersion', 'state_runtimeVersion', noParams]],
  ['state_subscribeStorage', ['state_unsubscribeStorage', 'state_storage', (params) => params.length === 1 && keysAt(params)]],
]);
const unsubscribeMethods = new Set([...subscriptions.values()].map(([method]) => method));
const allowedMethods = new Set([...reads.keys(), ...subscriptions.keys(), ...unsubscribeMethods]);

export function createGateway({ upstream, token }) {
  if (typeof token !== 'string' || !/^[0-9a-f]{64}$/.test(token)) throw new Error('Invalid gateway token');
  const endpoint = new URL(upstream);
  if (endpoint.protocol !== 'ws:' || !['127.0.0.1', '[::1]'].includes(endpoint.hostname)
    || !endpoint.port || endpoint.username || endpoint.password || endpoint.search || endpoint.hash || endpoint.pathname !== '/') {
    throw new Error('Upstream must be a numeric loopback WebSocket endpoint');
  }
  const secret = Buffer.from(token, 'hex');
  const server = createServer({ maxHeaderSize: 4096 }, (_request, response) => {
    response.writeHead(404, { Connection: 'close' });
    response.end();
  });
  server.maxConnections = MAX_CLIENTS * 2;
  server.requestTimeout = 10_000;
  server.headersTimeout = 5_000;
  const websocketServer = new WebSocketServer({ noServer: true, maxPayload: REQUEST_BYTES, perMessageDeflate: false });
  const sessions = new Set();
  const sockets = new Set();
  let closing = false;
  server.on('connection', (socket) => {
    sockets.add(socket);
    socket.setTimeout(10_000, () => socket.destroy());
    socket.on('close', () => sockets.delete(socket));
  });
  server.on('clientError', (_error, socket) => socket.destroy());
  server.on('upgrade', (request, socket, head) => {
    const match = /^\/rpc\/([0-9a-f]{64})$/.exec(request.url ?? '');
    const candidate = match ? Buffer.from(match[1], 'hex') : Buffer.alloc(32);
    const authenticated = timingSafeEqual(secret, candidate) && match !== null;
    if (!authenticated || closing || sessions.size >= MAX_CLIENTS) {
      socket.end(`HTTP/1.1 ${authenticated ? '503 Service Unavailable' : '401 Unauthorized'}\r\nConnection: close\r\nContent-Length: 0\r\n\r\n`);
      return;
    }
    socket.setTimeout(0);
    websocketServer.handleUpgrade(request, socket, head, (client) => {
      // A dedicated upstream connection makes closing it cancel every subscription owned by this client.
      const backend = new WebSocket(endpoint, { maxPayload: RESPONSE_BYTES, perMessageDeflate: false, handshakeTimeout: TIMEOUT_MS });
      const pending = new Map();
      const clientIds = new Set();
      const activeSubscriptions = new Map();
      let sequence = 0;
      let queuedBytes = 0;
      let pendingSubscriptions = 0;
      let stopped = false;
      const session = { client, backend, alive: true, cleanup };
      sessions.add(session);
      function cleanup() {
        if (stopped) return;
        stopped = true;
        sessions.delete(session);
        for (const request of pending.values()) clearTimeout(request.timer);
        pending.clear();
        clientIds.clear();
        activeSubscriptions.clear();
        client.terminate();
        backend.terminate();
      }
      function send(socket, value) {
        if (stopped || socket.readyState !== WebSocket.OPEN) return;
        const wire = JSON.stringify(value);
        if (Buffer.byteLength(wire) > RESPONSE_BYTES || socket.bufferedAmount + Buffer.byteLength(wire) > RESPONSE_BYTES) return cleanup();
        socket.send(wire, (error) => { if (error) cleanup(); });
      }
      function error(id, code, message) { send(client, { jsonrpc: '2.0', id, error: { code, message } }); }
      function forward(request, id) {
        const wire = JSON.stringify({ jsonrpc: '2.0', id, method: request.method, params: request.params });
        if (backend.bufferedAmount + Buffer.byteLength(wire) > REQUEST_BYTES * 2) return cleanup();
        backend.send(wire, (failure) => { if (failure) cleanup(); });
      }
      client.on('error', cleanup);
      client.on('close', cleanup);
      backend.on('error', cleanup);
      backend.on('close', cleanup);
      client.on('pong', () => { session.alive = true; });
      client.on('message', (data, binary) => {
        if (stopped) return;
        let request;
        try { if (binary) throw new Error(); request = JSON.parse(data.toString()); }
        catch { error(null, -32700, 'Invalid JSON request'); return; }
        if (!object(request) || request.jsonrpc !== '2.0' || !idValid(request.id)
          || typeof request.method !== 'string' || (request.params !== undefined && !Array.isArray(request.params))
          || Object.keys(request).some((key) => !['jsonrpc', 'id', 'method', 'params'].includes(key))) {
          error(object(request) && idValid(request.id) ? request.id : null, -32600, 'Expected one JSON-RPC request with an id and positional params');
          return;
        }
        if (clientIds.has(request.id)) return cleanup();
        if (!allowedMethods.has(request.method)) { error(request.id, -32601, 'Method not allowed'); return; }
        request.params ??= [];
        const subscription = subscriptions.get(request.method);
        const unsubscribe = unsubscribeMethods.has(request.method);
        const owned = unsubscribe && activeSubscriptions.get(request.params[0]);
        if (!(reads.get(request.method)?.(request.params) ?? subscription?.[2](request.params)
          ?? (request.params.length === 1 && owned && owned.unsubscribe === request.method && !owned.pending))) {
          error(request.id, -32602, 'Params not allowed');
          return;
        }
        if (pending.size >= MAX_INFLIGHT || (subscription && activeSubscriptions.size + pendingSubscriptions >= MAX_SUBSCRIPTIONS)) {
          error(request.id, -32000, 'Gateway capacity exceeded');
          return;
        }
        if (sequence === Number.MAX_SAFE_INTEGER) return cleanup();
        const id = ++sequence;
        const record = { ...request, subscription, owned, timer: setTimeout(() => {
          error(request.id, -32000, 'Upstream request timed out');
          cleanup();
        }, TIMEOUT_MS) };
        record.timer.unref();
        pending.set(id, record);
        clientIds.add(request.id);
        if (subscription) pendingSubscriptions++;
        if (owned) owned.pending = true;
        if (backend.readyState === WebSocket.OPEN) forward(record, id);
        else {
          queuedBytes += data.length;
          if (queuedBytes > REQUEST_BYTES * 2) cleanup();
        }
      });
      backend.on('open', () => {
        queuedBytes = 0;
        for (const [id, request] of pending) forward(request, id);
      });
      backend.on('message', (data, binary) => {
        if (stopped) return;
        let response;
        try { if (binary) throw new Error(); response = JSON.parse(data.toString()); }
        catch { cleanup(); return; }
        if (!object(response) || response.jsonrpc !== '2.0') return cleanup();
        if (Object.hasOwn(response, 'id')) {
          const record = pending.get(response.id);
          if (!record) return;
          const hasResult = Object.hasOwn(response, 'result');
          const hasError = Object.hasOwn(response, 'error');
          if (hasResult === hasError || (hasError && (!object(response.error) || !Number.isInteger(response.error.code) || typeof response.error.message !== 'string'))) return cleanup();
          clearTimeout(record.timer);
          pending.delete(response.id);
          clientIds.delete(record.id);
          if (record.subscription) {
            pendingSubscriptions--;
            if (hasResult) {
              if (!idValid(response.result) || activeSubscriptions.has(response.result)) return cleanup();
              activeSubscriptions.set(response.result, { unsubscribe: record.subscription[0], notification: record.subscription[1], pending: false });
            }
          }
          if (record.owned) {
            record.owned.pending = false;
            if (response.result === true) activeSubscriptions.delete(record.params[0]);
          }
          if (record.method === 'rpc_methods' && hasResult) {
            if (!object(response.result) || !Array.isArray(response.result.methods)) return cleanup();
            response.result = { version: response.result.version, methods: response.result.methods.filter((method) => allowedMethods.has(method)) };
          }
          send(client, { jsonrpc: '2.0', id: record.id, ...(hasResult ? { result: response.result } : { error: response.error }) });
        } else {
          const params = response.params;
          const owned = object(params) && activeSubscriptions.get(params.subscription);
          if (owned && owned.notification === response.method && Object.hasOwn(params, 'result')) {
            send(client, { jsonrpc: '2.0', method: response.method, params: { subscription: params.subscription, result: params.result } });
          }
        }
      });
    });
  });
  const heartbeat = setInterval(() => {
    for (const session of sessions) {
      if (!session.alive) session.cleanup();
      else {
        session.alive = false;
        if (session.client.readyState === WebSocket.OPEN) session.client.ping();
      }
    }
  }, TIMEOUT_MS);
  heartbeat.unref();
  const close = server.close;
  server.close = function (callback) {
    closing = true;
    clearInterval(heartbeat);
    for (const session of sessions) session.cleanup();
    for (const socket of sockets) socket.destroy();
    websocketServer.close();
    return close.call(this, callback);
  };
  server.on('error', () => server.close());
  return server;
}

function main() {
  const { values } = parseArgs({ options: { upstream: { type: 'string' }, port: { type: 'string' }, 'token-file': { type: 'string' } } });
  if (!values.upstream || !values['token-file'] || !/^[0-9]+$/.test(values.port ?? '')
    || Number(values.port) < 1 || Number(values.port) > 65535) throw new Error('Invalid gateway arguments');
  const fd = openSync(values['token-file'], constants.O_RDONLY | constants.O_NOFOLLOW);
  let token;
  try {
    const stat = fstatSync(fd);
    if (!stat.isFile() || stat.uid !== process.getuid() || (stat.mode & 0o077) !== 0 || stat.size > 65 || stat.size < 64) {
      throw new Error('Token file must be an owner-only regular file');
    }
    token = readFileSync(fd, 'utf8').replace(/\n$/, '');
  } finally { closeSync(fd); }
  const server = createGateway({ upstream: values.upstream, token });
  server.on('error', () => { console.error('Gateway failed'); process.exitCode = 1; });
  server.listen(Number(values.port), '127.0.0.1', () => console.log(`Read-only gateway listening on loopback port ${Number(values.port)}`));
  const shutdown = () => server.close();
  process.once('SIGINT', shutdown);
  process.once('SIGTERM', shutdown);
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  try { main(); }
  catch { console.error('Gateway configuration failed; use --upstream ws://127.0.0.1:PORT --port PORT --token-file OWNER_ONLY_FILE'); process.exitCode = 1; }
}
