import { FetchRequest, JsonRpcProvider } from 'ethers';
import type { Store } from '@subsquid/typeorm-store';
import { getEthereumEvidenceConfig } from '../common/env.js';
import { EthereumFinalityCheckpoint } from '../model/entities/ethereumFinalityCheckpoint.model.js';
import type { Receipt, ReceiptLog } from './completion.js';
import type { Context } from './processor.js';

export interface EthereumEvidenceClients {
  primaryCall(method: string, params: unknown[]): Promise<unknown>;
  witnessCall(method: string, params: unknown[]): Promise<unknown>;
  chainId: bigint;
  genesisHash: string;
}

export interface AgreedCheckpoint {
  schemaVersion: 1;
  policy: 'independent-two-el-v1';
  chainId: string;
  genesisHash: string;
  number: string;
  timestamp: string;
  hash: string;
  parentHash: string;
  stateRoot: string;
  transactionsRoot: string;
  receiptsRoot: string;
}

export interface AuthenticatedReceipt {
  receipt: Receipt;
  blockTimestamp: string;
  checkpoint: AgreedCheckpoint;
}

export type ReceiptAuthenticator = (
  hash: string, expected: { height: number; hash?: string },
) => Promise<AuthenticatedReceipt | undefined>;

export interface SourceReceiptAuth {
  version: 1;
  checkpointId: string;
  receipt: Receipt;
  blockTimestamp: string;
  localLogIndex: number;
}

type Header = Pick<AgreedCheckpoint,
  'number' | 'timestamp' | 'hash' | 'parentHash' | 'stateRoot' | 'transactionsRoot' | 'receiptsRoot'>;
type RpcCall = EthereumEvidenceClients['primaryCall'];
const RPC_TIMEOUT = 10_000;
const QUALIFICATION_TIMEOUT = 300_000;
const MAX_DATE_SECONDS = 8_640_000_000_000n;

function hold(message: string): never { throw new Error(`HOLD: ${message}`); }
function object(value: unknown): Record<string, unknown> {
  if (!value || typeof value !== 'object' || Array.isArray(value)) hold('missing or malformed Ethereum evidence');
  return value as Record<string, unknown>;
}
function bytes(value: unknown, length?: number): string {
  if (typeof value !== 'string' || !/^0x(?:[0-9a-f]{2})*$/i.test(value) ||
      (length !== undefined && value.length !== 2 + 2 * length)) hold('malformed Ethereum hex identity/data');
  return value.toLowerCase();
}
function quantity(value: unknown): string {
  if (typeof value !== 'string' || !/^0x[0-9a-f]+$/i.test(value)) hold('malformed Ethereum quantity');
  const number = BigInt(value);
  if (number > (1n << 256n) - 1n) hold('Ethereum quantity exceeds uint256');
  return `0x${number.toString(16)}`;
}
function safe(value: bigint): bigint {
  if (value < 0n || value > BigInt(Number.MAX_SAFE_INTEGER)) hold('Ethereum coordinate exceeds safe integer range');
  return value;
}
function height(value: number): bigint {
  if (!Number.isSafeInteger(value) || value < 0) hold('invalid expected Ethereum height');
  return BigInt(value);
}
function timestamp(value: bigint): string {
  if (value < 0n || value > MAX_DATE_SECONDS) hold('Ethereum timestamp exceeds Date range');
  return value.toString();
}
function header(value: unknown): Header {
  const raw = object(value);
  return {
    number: safe(BigInt(quantity(raw.number))).toString(),
    timestamp: timestamp(BigInt(quantity(raw.timestamp))),
    hash: bytes(raw.hash, 32), parentHash: bytes(raw.parentHash, 32), stateRoot: bytes(raw.stateRoot, 32),
    transactionsRoot: bytes(raw.transactionsRoot, 32), receiptsRoot: bytes(raw.receiptsRoot, 32),
  };
}
function sameHeader(a: Header, b: Header): boolean {
  return a.number === b.number && a.timestamp === b.timestamp && a.hash === b.hash && a.parentHash === b.parentHash &&
    a.stateRoot === b.stateRoot && a.transactionsRoot === b.transactionsRoot && a.receiptsRoot === b.receiptsRoot;
}
function decimal(value: unknown): bigint {
  if (typeof value !== 'string' || !/^(0|[1-9][0-9]*)$/.test(value)) hold('noncanonical checkpoint decimal');
  return BigInt(value);
}
function normalizedCheckpoint(value: unknown): AgreedCheckpoint {
  const raw = object(value);
  if (raw.schemaVersion !== 1 || raw.policy !== 'independent-two-el-v1') hold('unsupported Ethereum checkpoint policy');
  const chainId = decimal(raw.chainId);
  if (chainId > 0xffffffffffffffffn) hold('checkpoint chain ID exceeds uint64');
  const normalized: AgreedCheckpoint = {
    schemaVersion: 1, policy: 'independent-two-el-v1', chainId: chainId.toString(),
    genesisHash: bytes(raw.genesisHash, 32), number: safe(decimal(raw.number)).toString(),
    timestamp: timestamp(decimal(raw.timestamp)), hash: bytes(raw.hash, 32), parentHash: bytes(raw.parentHash, 32),
    stateRoot: bytes(raw.stateRoot, 32), transactionsRoot: bytes(raw.transactionsRoot, 32), receiptsRoot: bytes(raw.receiptsRoot, 32),
  };
  if (Object.keys(raw).length !== Object.keys(normalized).length ||
      Object.entries(normalized).some(([key, val]) => raw[key] !== val)) hold('noncanonical Ethereum checkpoint record');
  return normalized;
}
function approved(clients: EthereumEvidenceClients, checkpoint?: AgreedCheckpoint): void {
  if (typeof clients.primaryCall !== 'function' || typeof clients.witnessCall !== 'function' ||
      typeof clients.chainId !== 'bigint' || clients.chainId < 0n || clients.chainId > 0xffffffffffffffffn ||
      bytes(clients.genesisHash, 32) !== clients.genesisHash) hold('independent Ethereum witness/identity is not configured');
  if (checkpoint && (checkpoint.chainId !== clients.chainId.toString() || checkpoint.genesisHash !== clients.genesisHash)) {
    hold('retained Ethereum checkpoint belongs to another approved network');
  }
}
function remaining(deadline: number): number {
  const duration = deadline - Date.now();
  if (duration <= 0) hold('Ethereum ancestry qualification timed out');
  return Math.min(RPC_TIMEOUT, duration);
}
async function rpc(call: RpcCall, method: string, params: unknown[], deadline: number): Promise<unknown> {
  const timeout = remaining(deadline);
  let timer: NodeJS.Timeout | undefined;
  try {
    return await Promise.race([
      Promise.resolve().then(() => call(method, params)),
      new Promise<never>((_, reject) => {
        timer = setTimeout(() => reject(new Error(`HOLD: Ethereum RPC timed out (${method})`)), timeout);
      }),
    ]);
  } catch (error) {
    if (error instanceof Error && error.message.startsWith('HOLD:')) throw error;
    throw new Error(`HOLD: Ethereum RPC unavailable (${method})`, { cause: error });
  } finally { clearTimeout(timer); }
}
async function block(call: RpcCall, tag: string, deadline: number): Promise<unknown> {
  return rpc(call, 'eth_getBlockByNumber', [tag, false], deadline);
}
async function numbered(call: RpcCall, number: bigint, deadline: number): Promise<Header> {
  const result = header(await block(call, `0x${number.toString(16)}`, deadline));
  if (BigInt(result.number) !== number) hold('Ethereum header returned for the wrong height');
  return result;
}
async function agreedHeader(clients: EthereumEvidenceClients, number: bigint, deadline: number): Promise<Header> {
  const [primary, witness] = await Promise.all([
    numbered(clients.primaryCall, number, deadline), numbered(clients.witnessCall, number, deadline),
  ]);
  if (!sameHeader(primary, witness)) hold(`independent Ethereum headers disagree at ${number}`);
  return primary;
}
async function descend(call: RpcCall, tip: Header, ancestor: Header, deadline?: number): Promise<void> {
  let current = tip;
  if (BigInt(current.number) < BigInt(ancestor.number)) hold('finalized Ethereum head regressed');
  while (BigInt(current.number) > BigInt(ancestor.number)) {
    // Retained history is finite; bound each advancing request, not the checkpoint's age.
    const parent = await numbered(call, BigInt(current.number) - 1n, deadline ?? Date.now() + RPC_TIMEOUT);
    if (parent.hash !== current.parentHash) hold('broken finalized Ethereum ancestry');
    current = parent;
  }
  if (!sameHeader(current, ancestor)) hold('finalized Ethereum ancestry excludes the accepted checkpoint');
}
async function historicalHeader(
  clients: EthereumEvidenceClients, checkpoint: AgreedCheckpoint, target: bigint, deadline: number,
): Promise<Header> {
  if (target > BigInt(checkpoint.number)) hold('Ethereum target is not commonly finalized');
  // Under the two-provider policy, both canonical numbered headers authenticate a
  // historical target. Only retained finality fences require a complete ancestry walk.
  return agreedHeader(clients, target, deadline);
}
async function recheck(clients: EthereumEvidenceClients, checkpoint: AgreedCheckpoint, deadline: number): Promise<void> {
  if (!sameHeader(await agreedHeader(clients, BigInt(checkpoint.number), deadline), checkpoint)) {
    hold('common finalized Ethereum anchor changed during authentication');
  }
}

let transports: { key: string; witness: JsonRpcProvider; primary?: JsonRpcProvider } | undefined;

// CLI owners call this after their final transaction; live processors reuse transports.
export function destroyEthereumEvidenceClients(): void {
  transports?.primary?.destroy();
  transports?.witness.destroy();
  transports = undefined;
}

export function createEthereumEvidenceClients(primaryCall?: RpcCall): EthereumEvidenceClients {
  const config = getEthereumEvidenceConfig();
  const key = JSON.stringify([config.rpcUrl, config.witnessRpcUrl, config.chainId.toString(), config.genesisHash]);
  if (transports?.key !== key) {
    destroyEthereumEvidenceClients();
    const request = new FetchRequest(config.witnessRpcUrl);
    request.timeout = RPC_TIMEOUT;
    transports = {
      key, witness: new JsonRpcProvider(request, config.chainId, { staticNetwork: true, batchMaxCount: 1 }),
    };
  }
  if (!primaryCall && !transports.primary) {
    const request = new FetchRequest(config.rpcUrl);
    request.timeout = RPC_TIMEOUT;
    transports.primary = new JsonRpcProvider(request, config.chainId, { staticNetwork: true, batchMaxCount: 1 });
  }
  const { primary, witness } = transports;
  return {
    primaryCall: primaryCall ?? ((method, params) => primary!.send(method, params)),
    witnessCall: (method, params) => witness.send(method, params),
    chainId: config.chainId, genesisHash: config.genesisHash,
  };
}

export async function qualifyFinality(clients: EthereumEvidenceClients, previous?: AgreedCheckpoint): Promise<AgreedCheckpoint> {
  const deadline = Date.now() + QUALIFICATION_TIMEOUT;
  approved(clients, previous && normalizedCheckpoint(previous));
  const [primaryId, witnessId, primaryGenesis, witnessGenesis, primaryTip, witnessTip] = await Promise.all([
    rpc(clients.primaryCall, 'eth_chainId', [], deadline), rpc(clients.witnessCall, 'eth_chainId', [], deadline),
    numbered(clients.primaryCall, 0n, deadline), numbered(clients.witnessCall, 0n, deadline),
    block(clients.primaryCall, 'finalized', deadline).then(header), block(clients.witnessCall, 'finalized', deadline).then(header),
  ]);
  if (BigInt(quantity(primaryId)) !== clients.chainId || BigInt(quantity(witnessId)) !== clients.chainId ||
      primaryGenesis.hash !== clients.genesisHash || !sameHeader(primaryGenesis, witnessGenesis)) {
    hold('Ethereum providers do not match the approved chain identity');
  }
  const number = BigInt(primaryTip.number) < BigInt(witnessTip.number) ? BigInt(primaryTip.number) : BigInt(witnessTip.number);
  if (previous && number < BigInt(previous.number)) hold('common finalized Ethereum height regressed below retained checkpoint');
  const common = await agreedHeader(clients, number, deadline);
  await Promise.all([
    descend(clients.primaryCall, primaryTip, common, deadline), descend(clients.witnessCall, witnessTip, common, deadline),
  ]);
  const checkpoint: AgreedCheckpoint = {
    schemaVersion: 1, policy: 'independent-two-el-v1', chainId: clients.chainId.toString(), genesisHash: clients.genesisHash, ...common,
  };
  if (previous) {
    await Promise.all([
      descend(clients.primaryCall, checkpoint, previous),
      descend(clients.witnessCall, checkpoint, previous),
    ]);
  }
  await recheck(clients, checkpoint, Date.now() + RPC_TIMEOUT);
  return checkpoint;
}

function normalizeReceipt(value: unknown): Receipt {
  const raw = object(value);
  const receipt: Receipt = {
    transactionHash: bytes(raw.transactionHash, 32), blockHash: bytes(raw.blockHash, 32), blockNumber: quantity(raw.blockNumber),
    transactionIndex: quantity(raw.transactionIndex), type: quantity(raw.type), cumulativeGasUsed: quantity(raw.cumulativeGasUsed),
    logsBloom: bytes(raw.logsBloom, 256), status: quantity(raw.status), logs: [],
  };
  safe(BigInt(receipt.blockNumber)); safe(BigInt(receipt.transactionIndex));
  if (BigInt(receipt.type) > 0x7fn || !['0x0', '0x1'].includes(receipt.status) || !Array.isArray(raw.logs)) {
    hold('malformed Ethereum receipt outcome/type/logs');
  }
  let previousIndex: bigint | undefined;
  for (const value of raw.logs) {
    const item = object(value);
    if (item.removed !== undefined && item.removed !== false) hold('removed Ethereum receipt log');
    if (!Array.isArray(item.topics) || item.topics.length > 4) hold('malformed Ethereum log topics');
    const log: ReceiptLog = {
      address: bytes(item.address, 20), topics: item.topics.map((topic) => bytes(topic, 32)), data: bytes(item.data),
      logIndex: quantity(item.logIndex), transactionHash: bytes(item.transactionHash, 32), blockHash: bytes(item.blockHash, 32),
      blockNumber: quantity(item.blockNumber), transactionIndex: quantity(item.transactionIndex), removed: false,
    };
    const index = safe(BigInt(log.logIndex));
    if (log.transactionHash !== receipt.transactionHash || log.blockHash !== receipt.blockHash ||
        log.blockNumber !== receipt.blockNumber || log.transactionIndex !== receipt.transactionIndex ||
        (previousIndex !== undefined && index !== previousIndex + 1n)) hold('inconsistent or noncontiguous Ethereum receipt log coordinates');
    previousIndex = index;
    receipt.logs.push(log);
  }
  return receipt;
}

export async function authenticateReceipt(
  clients: EthereumEvidenceClients, txHash: string, expected: { height: number; hash?: string }, checkpoint: AgreedCheckpoint,
): Promise<AuthenticatedReceipt | undefined> {
  const deadline = Date.now() + QUALIFICATION_TIMEOUT;
  approved(clients, normalizedCheckpoint(checkpoint));
  const target = height(expected.height);
  const transactionHash = bytes(txHash, 32);
  const expectedHash = expected.hash === undefined ? undefined : bytes(expected.hash, 32);
  if (target > BigInt(checkpoint.number)) return undefined;
  const [primary, witness] = await Promise.all([
    rpc(clients.primaryCall, 'eth_getTransactionReceipt', [transactionHash], deadline),
    rpc(clients.witnessCall, 'eth_getTransactionReceipt', [transactionHash], deadline),
  ]);
  if (primary === null && witness === null) return undefined;
  if (primary === null || witness === null) hold('Ethereum providers disagree about original receipt presence');
  const receipt = normalizeReceipt(primary);
  if (JSON.stringify(receipt) !== JSON.stringify(normalizeReceipt(witness))) hold('independent Ethereum receipts disagree');
  if (receipt.transactionHash !== transactionHash || BigInt(receipt.blockNumber) !== target ||
      (expectedHash !== undefined && receipt.blockHash !== expectedHash)) hold('original Ethereum receipt identity changed');
  const inclusion = await historicalHeader(clients, checkpoint, target, deadline);
  if (inclusion.hash !== receipt.blockHash) hold('receipt is not in the canonical finalized Ethereum block');
  const blocks = await Promise.all([
    block(clients.primaryCall, `0x${target.toString(16)}`, deadline), block(clients.witnessCall, `0x${target.toString(16)}`, deadline),
  ]);
  for (const value of blocks) {
    const raw = object(value);
    if (!sameHeader(header(raw), inclusion) || !Array.isArray(raw.transactions) ||
        bytes(raw.transactions[Number(BigInt(receipt.transactionIndex))], 32) !== transactionHash) {
      hold('Ethereum receipt transaction index is not in the agreed block');
    }
  }
  await recheck(clients, checkpoint, deadline);
  return { receipt, blockTimestamp: inclusion.timestamp, checkpoint };
}

export function createReceiptAuthenticator(clients: EthereumEvidenceClients, checkpoint: AgreedCheckpoint): ReceiptAuthenticator {
  const receipts = new Map<string, Promise<AuthenticatedReceipt | undefined>>();
  return (hash, expected) => {
    const key = `${bytes(hash, 32)}:${height(expected.height)}:${expected.hash === undefined ? '' : bytes(expected.hash, 32)}`;
    let result = receipts.get(key);
    if (!result) {
      result = authenticateReceipt(clients, hash, expected, checkpoint);
      receipts.set(key, result);
    }
    return result;
  };
}

export function checkpointId(checkpoint: AgreedCheckpoint): string {
  const value = normalizedCheckpoint(checkpoint);
  return `${value.chainId}:${value.genesisHash}:${value.number}`;
}

export async function loadCheckpoint(store: Store, clients: EthereumEvidenceClients): Promise<AgreedCheckpoint | undefined> {
  approved(clients);
  // The database is not chain-partitioned: even a lower foreign row is a conflict.
  const rows = await store.find(EthereumFinalityCheckpoint, {});
  let highest: AgreedCheckpoint | undefined;
  for (const row of rows) {
    const checkpoint = normalizedCheckpoint(row.checkpoint);
    approved(clients, checkpoint);
    if (row.id !== checkpointId(checkpoint) || BigInt(row.blockNumber) !== BigInt(checkpoint.number)) hold('retained checkpoint row identity is inconsistent');
    if (!highest || BigInt(checkpoint.number) > BigInt(highest.number)) highest = checkpoint;
  }
  return highest;
}

export async function retainCheckpoint(store: Store, checkpoint: AgreedCheckpoint): Promise<void> {
  const normalized = normalizedCheckpoint(checkpoint);
  const id = checkpointId(normalized);
  const existing = await store.findOneBy(EthereumFinalityCheckpoint, { id });
  if (existing) {
    if (existing.id !== id || BigInt(existing.blockNumber) !== BigInt(normalized.number) ||
        JSON.stringify(normalizedCheckpoint(existing.checkpoint)) !== JSON.stringify(normalized)) hold('conflicting retained Ethereum checkpoint');
    return;
  }
  try {
    await store.insert(new EthereumFinalityCheckpoint({ id, blockNumber: BigInt(normalized.number), checkpoint: normalized }));
  } catch (error) {
    const databaseError = error as { code?: string; constraint?: string; driverError?: { code?: string; constraint?: string } };
    const cause = databaseError.driverError ?? databaseError;
    if (cause.code === '23505' && cause.constraint === 'ethereum_finality_checkpoint_pkey') {
      // PostgreSQL has aborted this transaction. Only the outer transaction owner may retry.
      throw Object.assign(new Error('Retry transaction after concurrent Ethereum checkpoint insertion', { cause: error }), { code: '40001' });
    }
    throw error;
  }
}

export async function requireFinalizedBatch(
  ctx: Context, headers: readonly { height: number; hash: string }[] = ctx.blocks.map(({ header }) => header),
): Promise<AgreedCheckpoint> {
  const clients = createEthereumEvidenceClients((method, params) => ctx._chain.client.call(method, params));
  const checkpoint = await qualifyFinality(clients, await loadCheckpoint(ctx.store, clients));
  const deadline = Date.now() + QUALIFICATION_TIMEOUT;
  const expected = new Map<bigint, string>();
  for (const item of headers) {
    const number = height(item.height);
    const hash = bytes(item.hash, 32);
    if (number > BigInt(checkpoint.number)) hold('Ethereum batch is ahead of common finality');
    if (expected.has(number) && expected.get(number) !== hash) hold('contradictory Ethereum batch headers');
    expected.set(number, hash);
  }
  let previous: Header | undefined;
  for (const [number, hash] of [...expected].sort(([a], [b]) => a < b ? -1 : 1)) {
    const current = await historicalHeader(clients, checkpoint, number, deadline);
    if (current.hash !== hash || (previous && BigInt(previous.number) + 1n === number && current.parentHash !== previous.hash)) {
      hold('noncanonical or unlinked Ethereum batch header');
    }
    previous = current;
  }
  await recheck(clients, checkpoint, deadline);
  return checkpoint;
}
