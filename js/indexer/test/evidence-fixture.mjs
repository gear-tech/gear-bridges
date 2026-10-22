import { createServer } from 'node:http';
import { readFile } from 'node:fs/promises';
import { fileURLToPath } from 'node:url';
import { Interface } from 'ethers';

export const addr = (byte) => `0x${byte.repeat(20)}`;
export const hash = (byte) => `0x${byte.repeat(32)}`;
const hex = (value) => `0x${value.toString(16)}`;
export const fixtureIdentity = {
  manager: addr('11'), queue: addr('22'), sender: hash('44'), receiver: addr('55'),
  token: addr('66'), varaToken: hash('77'), message: hash('88'),
};

// Two local routes exercise independent-response checks, not provider independence or MPT verification.
export async function createEvidenceFixture() {
  const load = async (name) => new Interface(JSON.parse(await readFile(new URL(`../../../api/ethereum/${name}.json`, import.meta.url), 'utf8')).abi);
  const manager = await load('IERC20Manager');
  const queue = await load('IMessageQueue');
  const txHashes = ['b0', 'b1', 'b2', 'b3'].map(hash);
  const headers = Array.from({ length: 14 }, (_, number) => ({
    number: hex(number), hash: hash(number.toString(16).padStart(2, '0')),
    parentHash: hash(Math.max(0, number - 1).toString(16).padStart(2, '0')),
    timestamp: hex(1_700_000_000 + number), stateRoot: hash('a1'),
    transactionsRoot: hash('a2'), receiptsRoot: hash('a3'),
    transactions: number === 11 ? txHashes : [],
  }));
  const event = (abi, name, values, address, transactionIndex, logIndex) => ({
    ...abi.encodeEventLog(abi.getEvent(name), values), address,
    transactionHash: txHashes[transactionIndex], blockHash: headers[11].hash,
    blockNumber: '0xb', transactionIndex: hex(transactionIndex), logIndex: hex(logIndex), removed: false,
  });
  const { sender, receiver, token, message } = fixtureIdentity;
  const receipts = txHashes.map((transactionHash, index) => ({
    transactionHash, blockHash: headers[11].hash, blockNumber: '0xb', transactionIndex: hex(index),
    type: '0x2', status: '0x1', cumulativeGasUsed: hex(21_000 * (index + 1)), logsBloom: `0x${'00'.repeat(256)}`,
    logs: index < 2 ? [
      event(manager, 'Bridged', [sender, receiver, token, 10n], fixtureIdentity.manager, index, index * 2),
      event(queue, 'MessageProcessed', [9n, message, BigInt(8 + index), fixtureIdentity.manager], fixtureIdentity.queue, index, index * 2 + 1),
    ] : [event(manager, 'BridgingRequested', [receiver, sender, token, 10n], fixtureIdentity.manager, index, index + 2)],
  }));
  const state = { primaryHeight: 12, witnessHeight: 11, fault: undefined, requests: [] };
  const servers = [];
  const serve = async (side) => {
    const server = createServer(async (request, response) => {
      try {
        let body = '';
        for await (const chunk of request) { body += chunk; if (body.length > 1_000_000) throw new Error('oversized RPC request'); }
        const answer = async ({ id, method, params = [] }) => {
          state.requests.push({ side, method, params });
          let result;
          if (method === 'eth_chainId') result = '0x539';
          else if (method === 'eth_getBlockByNumber') result = headers[params[0] === 'finalized'
            ? state[`${side}Height`] : Number(BigInt(params[0]))] ?? null;
          else if (method === 'eth_getBlockByHash') result = headers.find((header) => header.hash === params[0]) ?? null;
          else if (method === 'eth_getTransactionReceipt') result = receipts.find((receipt) => receipt.transactionHash === params[0]) ?? null;
          else throw new Error(`Unexpected fixture RPC ${method}`);
          result = structuredClone(result);
          if (state.fault) result = await state.fault({ side, method, params, result });
          return { jsonrpc: '2.0', id, result };
        };
        const input = JSON.parse(body);
        const result = Array.isArray(input) ? await Promise.all(input.map(answer)) : await answer(input);
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
  try {
    const rpcUrl = await serve('primary');
    const witnessRpcUrl = await serve('witness');
    return {
      state, headers, receipts, txHashes,
      env: { ETH_RPC_URL: rpcUrl, ETH_WITNESS_RPC: witnessRpcUrl, ETH_CHAIN_ID: '1337',
        ETH_GENESIS_HASH: headers[0].hash, ETH_ERC20_MANAGER: fixtureIdentity.manager, ETH_MSQ_QUEUE: fixtureIdentity.queue,
        ETH_API_PATH: fileURLToPath(new URL('../../../api/ethereum', import.meta.url)) },
      async close() {
        await Promise.all(servers.map((server) => new Promise((resolve) => { server.close(resolve); server.closeAllConnections(); })));
      },
    };
  } catch (error) { for (const server of servers) server.close(); throw error; }
}

// Keep this process alive while running the plan's standalone repair commands in another shell.
if (process.argv[1] === fileURLToPath(import.meta.url)) {
  const fixture = await createEvidenceFixture();
  fixture.state.witnessHeight = 12; // The completed DB smoke retains checkpoint 12.
  console.log(Object.entries(fixture.env).map(([key, value]) => `export ${key}='${value}'`).join('\n'));
  const stop = async () => { await fixture.close(); process.exit(0); };
  process.once('SIGINT', stop);
  process.once('SIGTERM', stop);
}
