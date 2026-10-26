import { createServer } from 'node:http';
import { execFile } from 'node:child_process';
import { promisify } from 'node:util';
import { mkdtemp, mkdir, readdir, readFile, writeFile, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { Interface } from 'ethers';

export const addr = (byte) => `0x${byte.repeat(20)}`;
export const hash = (byte) => `0x${byte.repeat(32)}`;
export const fixtureIdentity = {
  manager: addr('11'), queue: addr('22'), sender: hash('44'), receiver: addr('55'),
  token: addr('66'), varaToken: hash('77'), message: hash('88'),
};
const execute = promisify(execFile);

// Only the test-only Rust exporter signs scenarios. JavaScript serves its unchanged
// Beacon/EL responses; the production native child performs every authorization.
export async function createEvidenceFixture(options = {}) {
  const exporter = process.env.ETH_EVIDENCE_FIXTURE_BINARY;
  const binary = process.env.ETH_EVIDENCE_BINARY;
  if (!exporter || !binary) throw new Error('Set ETH_EVIDENCE_FIXTURE_BINARY and ETH_EVIDENCE_BINARY to the built native executables');
  const directory = await mkdtemp(join(tmpdir(), 'indexer-native-evidence-'));
  const servers = [];
  const owned = new Set();
  let priorEnv;
  try {
    let specifications = options.receipts;
    if (!specifications) {
      const load = async (name) => new Interface(JSON.parse(await readFile(new URL(`../../../api/ethereum/${name}.json`, import.meta.url), 'utf8')).abi);
      const manager = await load('IERC20Manager'), queue = await load('IMessageQueue');
      const event = (abi, name, values, address) => ({ ...abi.encodeEventLog(abi.getEvent(name), values), address });
      const { sender, receiver, token, message } = fixtureIdentity;
      specifications = Array.from({ length: 4 }, (_, index) => ({ blockNumber: 11, transactionIndex: index,
        logs: index < 2 ? [
          event(manager, 'Bridged', [sender, receiver, token, 10n], fixtureIdentity.manager),
          event(queue, 'MessageProcessed', [9n, message, BigInt(8 + index), fixtureIdentity.manager], fixtureIdentity.queue),
          ...(index === 0 ? [event(queue, 'MerkleRoot', [12n, hash('f1')], fixtureIdentity.queue)] : []),
        ] : [event(manager, 'BridgingRequested', [receiver, sender, token, 10n], fixtureIdentity.manager)],
      }));
    }
    const finalizedHeight = options.finalizedHeight ?? 11;
    const finalizedHeights = options.finalizedHeights ?? [0, 10, 11, 12, 13];
    const complete = specifications.map((receipt) => ({
      blockNumber: Number(BigInt(receipt.blockNumber)), transactionIndex: Number(BigInt(receipt.transactionIndex)),
      logs: receipt.logs.map(({ address, topics, data }) => ({ address, topics, data })),
      ...(receipt.status === undefined ? {} : { status: receipt.status === true || BigInt(receipt.status) === 1n }),
    }));
    for (const receipt of [...complete]) for (let index = 0; index < receipt.transactionIndex; index++) {
      if (!complete.some((other) => other.blockNumber === receipt.blockNumber && other.transactionIndex === index)) {
        complete.push({ blockNumber: receipt.blockNumber, transactionIndex: index, logs: [] });
      }
    }
    complete.sort((a, b) => a.blockNumber - b.blockNumber || a.transactionIndex - b.transactionIndex);
    const specification = { receipts: complete, finalizedHeight, finalizedHeights };
    const input = join(directory, 'scenario.json'), output = join(directory, 'bundle.json');
    await writeFile(input, JSON.stringify(specification));
    await execute(exporter, ['--input', input, '--output', output], { maxBuffer: 1024 * 1024 });
    const bundle = JSON.parse(await readFile(output, 'utf8'));
    const headers = bundle.executionBlocks;
    const receipts = bundle.receipts;
    const txHashes = specifications.map((spec) => receipts.find((receipt) =>
      BigInt(receipt.blockNumber) === BigInt(spec.blockNumber) && BigInt(receipt.transactionIndex) === BigInt(spec.transactionIndex)).transactionHash);
    const bootstrap = join(directory, 'bootstrap.json');
    await writeFile(bootstrap, typeof bundle.approvalFileBytes === 'string'
      ? Buffer.from(bundle.approvalFileBytes.slice(2), 'hex') : Buffer.from(bundle.approvalFileBytes));
    const state = { finalizedHeight, fault: undefined, requests: [] };
    const respond = async (side, method, params, result) => {
      state.requests.push({ side, method, params });
      result = structuredClone(result);
      return state.fault ? state.fault({ side, method, params, result }) : result;
    };
    const call = async (method, params = []) => {
      let result;
      if (method === 'eth_chainId') result = '0x7a69';
      else if (method === 'eth_blockNumber') result = `0x${state.finalizedHeight.toString(16)}`;
      else if (method === 'eth_getBlockByNumber') result = headers[params[0] === 'finalized' || params[0] === 'latest'
        ? state.finalizedHeight : Number(BigInt(params[0]))] ?? null;
      else if (method === 'eth_getBlockByHash') result = headers.find((header) => header.hash === params[0]) ?? null;
      else if (method === 'eth_getBlockReceipts') {
        const selected = typeof params[0] === 'object' ? params[0].blockHash : params[0];
        const header = selected.length === 66 ? headers.find((header) => header.hash === selected) : headers[Number(BigInt(selected))];
        result = header ? receipts.filter((receipt) => receipt.blockHash === header.hash) : null;
      } else if (method === 'eth_getTransactionReceipt') result = receipts.find((receipt) => receipt.transactionHash === params[0]) ?? null;
      else throw new Error(`Unexpected fixture RPC ${method}`);
      result = await respond('primary', method, params, result);
      if (result && (method === 'eth_getBlockByNumber' || method === 'eth_getBlockByHash') && !params[1]) {
        result.transactions = result.transactions.map((transaction) => typeof transaction === 'string' ? transaction : transaction.hash);
      }
      return result;
    };
    const serve = async (side) => {
      const server = createServer(async (request, response) => {
        try {
          let result;
          if (side === 'beacon') {
            const route = request.url;
            if (route.startsWith('/eth/v1/beacon/light_client/bootstrap/')) result = bundle.bootstrap;
            else if (route.startsWith('/eth/v2/beacon/blocks/')) result = bundle.beaconBlocksByRoot[route.split('/').at(-1)];
            else if (route.startsWith('/eth/v1/beacon/light_client/updates')) result = bundle.updates;
            else if (route === '/eth/v1/beacon/light_client/finality_update') result = bundle.finalityUpdatesByExecutionNumber[String(state.finalizedHeight)];
            else throw new Error(`Unexpected fixture Beacon route ${route}`);
            if (!result) throw new Error(`No signed fixture response for ${route} at ${state.finalizedHeight}`);
            result = await respond('beacon', route, [], result);
          } else {
            let body = '';
            for await (const chunk of request) { body += chunk; if (body.length > 1_000_000) throw new Error('oversized RPC request'); }
            const answer = async ({ id, method, params = [] }) => ({ jsonrpc: '2.0', id, result: await call(method, params) });
            const input = JSON.parse(body);
            result = Array.isArray(input) ? await Promise.all(input.map(answer)) : await answer(input);
          }
          response.writeHead(200, { 'content-type': 'application/json' });
          response.end(JSON.stringify(result));
        } catch (error) {
          response.writeHead(503, { 'content-type': 'application/json' });
          response.end(JSON.stringify({ error: String(error) }));
        }
      });
      servers.push(server);
      await new Promise((resolve, reject) => { server.once('error', reject); server.listen(0, '127.0.0.1', resolve); });
      return `http://127.0.0.1:${server.address().port}`;
    };
    const rpcUrl = await serve('primary'), beaconRpc = await serve('beacon');
    const env = { ETH_RPC_URL: rpcUrl, ETH_EVIDENCE_BINARY: binary, ETH_CHAIN_ID: '31337',
      ETH_GENESIS_HASH: headers[0].hash, ETH_FINALITY_BEACON_RPC: beaconRpc,
      ETH_FINALITY_BOOTSTRAP: bootstrap, ETH_FINALITY_BOOTSTRAP_SHA256: bundle.approvalSha256,
      ETH_FINALITY_ARCHIVE_ROOT: join(directory, 'archive'), ETH_ERC20_MANAGER: fixtureIdentity.manager,
      ETH_MSQ_QUEUE: fixtureIdentity.queue, ETH_API_PATH: fileURLToPath(new URL('../../../api/ethereum', import.meta.url)) };
    const fixture = { state, headers, receipts, txHashes, env, directory, bundle, call,
      install() {
        priorEnv ??= Object.fromEntries(Object.keys(env).map((key) => [key, process.env[key]]));
        Object.assign(process.env, env);
        for (const owner of ['eth', 'gear', 'repair']) owned.add(owner);
      },
      async clients(owner = 'eth') {
        fixture.install();
        const { createEthereumEvidenceClients } = await import('../lib/eth/finality.js');
        owned.add(owner);
        return createEthereumEvidenceClients(owner);
      },
      async checkpoint(height = state.finalizedHeight, owner = 'repair') {
        state.finalizedHeight = height;
        const { qualifyFinality } = await import('../lib/eth/finality.js');
        return qualifyFinality(await fixture.clients(owner));
      },
      async corruptProof(transactionHash, owner = 'eth') {
        const store = join(env.ETH_FINALITY_ARCHIVE_ROOT, owner);
        const digest = JSON.parse(await readFile(join(store, `original-${transactionHash.slice(2)}.json`), 'utf8'));
        await writeFile(join(store, `evidence-${digest.replace(/^0x/, '')}.json`), '{}');
      },
      async removeProof(transactionHash, owner = 'eth') {
        const store = join(env.ETH_FINALITY_ARCHIVE_ROOT, owner);
        const digest = JSON.parse(await readFile(join(store, `original-${transactionHash.slice(2)}.json`), 'utf8'));
        await rm(join(store, `evidence-${digest.replace(/^0x/, '')}.json`));
        // A missing cache alone is recoverable: deny the original full block too.
        const blockHash = receipts.find((receipt) => receipt.transactionHash === transactionHash).blockHash;
        const priorFault = state.fault;
        state.fault = (request) => {
          if (request.side === 'primary' && request.params[1] === true && request.result?.hash === blockHash &&
              (request.method === 'eth_getBlockByHash' || request.method === 'eth_getBlockByNumber')) return null;
          return priorFault ? priorFault(request) : request.result;
        };
      },
      async freshArchiveRoot() {
        const root = await mkdtemp(join(directory, 'negative-archive-'));
        const target = join(root, 'eth', 'beacon');
        await mkdir(target, { recursive: true });
        for (const owner of ['eth', 'gear', 'repair']) {
          const source = join(env.ETH_FINALITY_ARCHIVE_ROOT, owner, 'beacon');
          const names = await readdir(source).catch((error) => {
            if (error.code === 'ENOENT') return [];
            throw error;
          });
          for (const name of names.filter((name) => /^proof-[0-9a-f]{64}\.json$/.test(name))) {
            const bytes = await readFile(join(source, name));
            try { await writeFile(join(target, name), bytes, { flag: 'wx' }); }
            catch (error) {
              if (error.code !== 'EEXIST' || !bytes.equals(await readFile(join(target, name)))) throw error;
            }
          }
        }
        return root;
      },
      async close() {
        const { destroyEthereumEvidenceClients } = await import('../lib/eth/finality.js');
        for (const owner of owned) destroyEthereumEvidenceClients(owner);
        await Promise.all(servers.map((server) => new Promise((resolve) => { server.close(resolve); server.closeAllConnections(); })));
        if (priorEnv) for (const [key, value] of Object.entries(priorEnv)) {
          if (process.env[key] !== env[key]) continue;
          if (value === undefined) delete process.env[key]; else process.env[key] = value;
        }
        await rm(directory, { recursive: true, force: true });
      },
    };
    return fixture;
  } catch (error) {
    for (const server of servers) { server.close(); server.closeAllConnections(); }
    await rm(directory, { recursive: true, force: true });
    throw error;
  }
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  const fixture = await createEvidenceFixture();
  console.log(Object.entries(fixture.env).map(([key, value]) => `export ${key}='${value}'`).join('\n'));
  const stop = async () => { await fixture.close(); process.exit(0); };
  process.once('SIGINT', stop);
  process.once('SIGTERM', stop);
}
