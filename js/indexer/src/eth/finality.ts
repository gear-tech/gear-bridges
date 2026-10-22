import { isDeepStrictEqual } from 'node:util';
import { mkdirSync, realpathSync } from 'node:fs';
import { join } from 'node:path';
import { keccak256, toUtf8Bytes } from 'ethers';
import { NativeEvidenceClient, EthereumEvidenceError, exactObject } from './native-evidence.js';
import type { Store } from '@subsquid/typeorm-store';
import { getEthereumEvidenceConfig, type EvidenceOwner } from '../common/env.js';
import { EthereumFinalityCheckpoint } from '../model/entities/ethereumFinalityCheckpoint.model.js';
import type { Receipt, ReceiptLog } from './completion.js';
import type { Context } from './processor.js';

export interface EthereumEvidenceClients {
  native: NativeEvidenceClient;
  chainId: bigint;
  genesisHash: string;
}

export interface HeaderEvidence {
  number: string;
  timestamp: string;
  hash: string;
  parentHash: string;
  stateRoot: string;
  transactionsRoot: string;
  receiptsRoot: string;
}
export interface LegacyCheckpoint extends HeaderEvidence {
  schemaVersion: 1;
  policy: 'independent-two-el-v1';
  chainId: string;
  genesisHash: string;
}
export interface NativeCheckpoint {
  schema_version: 2;
  policy: 'beacon-sync-committee-v1';
  chain_id: number;
  genesis_hash: string;
  common_header: Record<string, unknown>;
  beacon: { genesisValidatorsRoot: string; bootstrapApprovalHash: string; finalizedBeaconHeaderScale: string; proofDigest: string };
}
export interface RequalificationLink { originalDigest: string; proofDigest: string }
export interface RetainedRequalificationLink extends RequalificationLink { checkpoint: NativeCheckpoint }
export interface AgreedCheckpoint extends HeaderEvidence {
  schemaVersion: 2;
  policy: 'beacon-sync-committee-v1';
  chainId: string;
  genesisHash: string;
  native: NativeCheckpoint;
}
export type HistoricalCheckpoint = LegacyCheckpoint | AgreedCheckpoint;

export interface ProofReference {
  inclusionProofDigest: string;
  checkpointProofDigest: string;
  requalificationLinks: RequalificationLink[];
}

export interface AuthenticatedReceipt extends ProofReference {
  receipt: Receipt;
  blockTimestamp: string;
  checkpoint: AgreedCheckpoint;
}

export type ReceiptAuthenticator = (
  hash: string, expected: { height: number; hash?: string; proofDigest?: string },
) => Promise<AuthenticatedReceipt | undefined>;

export interface SourceReceiptAuth extends ProofReference {
  version: 2;
  checkpointId: string;
  receipt: Receipt;
  blockTimestamp: string;
  localLogIndex: number;
}

type Header = HeaderEvidence;
const MAX_DATE_SECONDS = 8_640_000_000_000n;

function hold(message: string): never { throw new EthereumEvidenceError('conflict', message); }
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
function header(value: unknown): Omit<Header, 'hash'> {
  const raw = object(value);
  return {
    number: safe(BigInt(quantity(raw.number))).toString(),
    timestamp: timestamp(BigInt(quantity(raw.timestamp))),
    parentHash: bytes(raw.parentHash, 32), stateRoot: bytes(raw.stateRoot, 32),
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
export function normalizedCheckpoint(value: unknown): HistoricalCheckpoint {
  const raw = object(value);
  const legacy = raw.schemaVersion === 1 && raw.policy === 'independent-two-el-v1';
  const beacon = raw.schemaVersion === 2 && raw.policy === 'beacon-sync-committee-v1';
  if (!legacy && !beacon) hold('unsupported Ethereum checkpoint policy');
  const chainId = decimal(raw.chainId);
  if (chainId > 0xffffffffffffffffn) hold('checkpoint chain ID exceeds uint64');
  const common = {
    chainId: chainId.toString(), genesisHash: bytes(raw.genesisHash, 32),
    number: safe(decimal(raw.number)).toString(), timestamp: timestamp(decimal(raw.timestamp)),
    hash: bytes(raw.hash, 32), parentHash: bytes(raw.parentHash, 32), stateRoot: bytes(raw.stateRoot, 32),
    transactionsRoot: bytes(raw.transactionsRoot, 32), receiptsRoot: bytes(raw.receiptsRoot, 32),
  };
  const normalized: HistoricalCheckpoint = legacy
    ? { schemaVersion: 1, policy: 'independent-two-el-v1', ...common }
    : { schemaVersion: 2, policy: 'beacon-sync-committee-v1', ...common, native: nativeCheckpoint(raw.native) };
  if (!isDeepStrictEqual(raw, normalized)) hold('noncanonical Ethereum checkpoint record');
  if (normalized.schemaVersion === 2) {
    // Consensus-header JSON has no RPC hash. The native qualification below
    // authenticates the separately retained hash, not this metadata projection.
    if (Object.entries(header(normalized.native.common_header))
        .some(([key, value]) => normalized[key as keyof Header] !== value) ||
        String(normalized.native.chain_id) !== normalized.chainId || normalized.native.genesis_hash !== normalized.genesisHash) {
      hold('native checkpoint projection mismatch');
    }
  }
  return normalized;
}

export function legacyCheckpointDigest(value: unknown): string {
  const raw = object(value);
  if (normalizedCheckpoint(raw).schemaVersion !== 1) hold('legacy metadata digest requires the original schema-one record');
  // Metadata identity only, not finality authority. Matches the native canonical
  // compact JSON: sorted ASCII keys, original string values, no whitespace.
  return keccak256(toUtf8Bytes(JSON.stringify(Object.fromEntries(Object.keys(raw).sort().map((key) => [key, raw[key]])))));
}

function nativeCheckpoint(value: unknown): NativeCheckpoint {
  const raw = exactObject(value, ['schema_version', 'policy', 'chain_id', 'genesis_hash', 'common_header', 'beacon']);
  if (raw.schema_version !== 2 || raw.policy !== 'beacon-sync-committee-v1' ||
      !Number.isSafeInteger(raw.chain_id) || Number(raw.chain_id) < 0 ||
      bytes(raw.genesis_hash, 32) !== raw.genesis_hash) hold('invalid native checkpoint identity');
  const beacon = exactObject(raw.beacon, ['genesisValidatorsRoot', 'bootstrapApprovalHash', 'finalizedBeaconHeaderScale', 'proofDigest']);
  for (const key of ['genesisValidatorsRoot', 'bootstrapApprovalHash', 'proofDigest']) {
    if (bytes(beacon[key], 32) !== beacon[key]) hold('invalid native checkpoint proof reference');
  }
  if (bytes(beacon.finalizedBeaconHeaderScale) !== beacon.finalizedBeaconHeaderScale ||
      beacon.finalizedBeaconHeaderScale === '0x') hold('missing native Beacon header');
  header(raw.common_header);
  return raw as unknown as NativeCheckpoint;
}

function approved(clients: EthereumEvidenceClients, checkpoint?: HistoricalCheckpoint): void {
  if (!(clients.native instanceof NativeEvidenceClient) || typeof clients.chainId !== 'bigint' ||
      clients.chainId < 0n || clients.chainId > 0xffffffffffffffffn ||
      bytes(clients.genesisHash, 32) !== clients.genesisHash) hold('native Ethereum authority is not configured');
  if (checkpoint && (checkpoint.chainId !== clients.chainId.toString() || checkpoint.genesisHash !== clients.genesisHash)) {
    hold('retained Ethereum checkpoint belongs to another approved network');
  }
}

const transports = new Map<EvidenceOwner, { key: string; clients: EthereumEvidenceClients }>();
export function destroyEthereumEvidenceClients(owner?: EvidenceOwner): void {
  for (const [name, transport] of transports) {
    if (owner !== undefined && owner !== name) continue;
    transport.clients.native.close();
    transports.delete(name);
  }
}

export function createEthereumEvidenceClients(owner: EvidenceOwner = 'eth'): EthereumEvidenceClients {
  const config = getEthereumEvidenceConfig(owner);
  const key = JSON.stringify({ ...config, chainId: config.chainId.toString() });
  let transport = transports.get(owner);
  if (transport?.key !== key) {
    destroyEthereumEvidenceClients(owner);
    for (const directory of [...config.proofSources, join(config.store, 'beacon')]) mkdirSync(directory, { recursive: true });
    const native = new NativeEvidenceClient(config.binary, ['ethereum-evidence',
      '--ethereum-endpoint', config.rpcUrl, '--ethereum-finality-beacon-rpc', config.beaconRpc,
      '--ethereum-finality-bootstrap', config.bootstrap, '--ethereum-finality-bootstrap-sha256', config.bootstrapSha256,
      '--ethereum-finality-store', config.store,
      ...config.proofSources.flatMap((path) => ['--ethereum-finality-proof-source', realpathSync(path)]),
      ...config.evidenceProofSources.flatMap((path) => ['--ethereum-evidence-proof-source', realpathSync(path)])]);
    transport = { key, clients: { native, chainId: config.chainId, genesisHash: config.genesisHash } };
    transports.set(owner, transport);
  }
  return transport!.clients;
}

function links(value: unknown): RequalificationLink[] {
  if (!Array.isArray(value)) hold('missing native requalification links');
  const seen = new Set<string>();
  return value.map((item) => {
    const raw = exactObject(item, ['originalDigest', 'proofDigest']);
    const link = { originalDigest: bytes(raw.originalDigest, 32), proofDigest: bytes(raw.proofDigest, 32) };
    if (!isDeepStrictEqual(link, raw) || seen.has(link.originalDigest)) hold('invalid duplicate requalification link');
    seen.add(link.originalDigest);
    return link;
  });
}

export async function qualifyFinality(
  clients: EthereumEvidenceClients, previous?: HistoricalCheckpoint,
  targets: readonly { number: string; hash: string }[] = [],
  retained: readonly HistoricalCheckpoint[] = [],
  retainedLinks: readonly RetainedRequalificationLink[] = [],
): Promise<AgreedCheckpoint> {
  approved(clients, previous && normalizedCheckpoint(previous));
  for (const link of retainedLinks) {
    const native = nativeCheckpoint(link.checkpoint);
    if (String(native.chain_id) !== clients.chainId.toString() || native.genesis_hash !== clients.genesisHash) {
      hold('retained requalification belongs to another approved network');
    }
  }
  const fences = new Map<string, string>();
  for (const target of [...targets, ...retained, ...(previous ? [previous] : [])]) {
    const number = safe(decimal(target.number)).toString(), hash = bytes(target.hash, 32);
    if (fences.has(number) && fences.get(number) !== hash) hold('contradictory Ethereum checkpoint fences');
    fences.set(number, hash);
  }
  for (const checkpoint of retained) approved(clients, normalizedCheckpoint(checkpoint));
  const wire = (checkpoint: HistoricalCheckpoint) => checkpoint.schemaVersion === 2 ? checkpoint.native : checkpoint;
  const response = await clients.native.request({ op: 'qualify',
    ...(previous ? { previousCheckpoint: wire(previous) } : {}),
    ...(retained.length ? { checkpointFences: retained.map(wire) } : {}),
    ...(retainedLinks.length ? { retainedRequalificationLinks: retainedLinks } : {}),
    targets: [...fences].map(([number, hash]) => ({ number, hash })) });
  exactObject(response, ['id', 'ok', 'op', 'checkpoint', 'headers', 'requalificationLinks']);
  if (response.op !== 'qualify' || !Array.isArray(response.headers)) hold('wrong native qualification response');
  const native = nativeCheckpoint(response.checkpoint);
  const common = header(native.common_header);
  const headers = new Map<string, Header>();
  for (const item of response.headers) {
    const raw = exactObject(item, ['number', 'timestamp', 'hash', 'parentHash', 'stateRoot', 'transactionsRoot', 'receiptsRoot', 'proofDigest']);
    const current: Header = { number: safe(decimal(raw.number)).toString(), timestamp: timestamp(decimal(raw.timestamp)),
      hash: bytes(raw.hash, 32), parentHash: bytes(raw.parentHash, 32), stateRoot: bytes(raw.stateRoot, 32),
      transactionsRoot: bytes(raw.transactionsRoot, 32), receiptsRoot: bytes(raw.receiptsRoot, 32) };
    if (!isDeepStrictEqual({ ...current, proofDigest: bytes(raw.proofDigest, 32) }, raw) ||
        headers.has(current.number)) hold('unexpected native authenticated header');
    headers.set(current.number, current);
  }
  for (const link of retainedLinks) {
    const retainedHeader = header(link.checkpoint.common_header);
    const authenticated = headers.get(retainedHeader.number);
    if (!authenticated || Object.entries(retainedHeader).some(([key, value]) => authenticated[key as keyof Header] !== value) ||
        (fences.has(retainedHeader.number) && fences.get(retainedHeader.number) !== authenticated.hash)) {
      hold('missing or inconsistent retained link checkpoint header');
    }
    // The native child hashes and authenticates the full consensus header. Its
    // separately returned hash also belongs to the exact requested fence set.
    fences.set(retainedHeader.number, authenticated.hash);
  }
  const head = headers.get(common.number);
  if (!head || Object.entries(common).some(([key, value]) => head[key as keyof Header] !== value)) {
    hold('missing or inconsistent native finalized header');
  }
  const checkpoint: AgreedCheckpoint = { schemaVersion: 2, policy: 'beacon-sync-committee-v1',
    chainId: String(native.chain_id), genesisHash: native.genesis_hash, ...head, native };
  approved(clients, checkpoint);
  if (fences.has(checkpoint.number) && fences.get(checkpoint.number) !== checkpoint.hash) {
    hold('native finalized header contradicts a requested fence');
  }
  fences.set(checkpoint.number, checkpoint.hash);
  if (headers.size !== fences.size ||
      [...fences].some(([number, hash]) => headers.get(number)?.hash !== hash)) {
    hold('incomplete native fence qualification');
  }
  for (const retainedCheckpoint of [...retained, ...(previous ? [previous] : [])]) {
    if (!sameHeader(headers.get(retainedCheckpoint.number)!, retainedCheckpoint)) hold('retained checkpoint fields changed');
  }
  const requalified = links(response.requalificationLinks);
  const expectedLinks = [...retained, ...(previous ? [previous] : [])].filter((item) => item.schemaVersion === 1);
  if (requalified.length !== new Set(expectedLinks.map((item) => checkpointId(item))).size) hold('missing legacy requalification link');
  qualifications.set(checkpoint, requalified);
  const verified = new Map<string, HistoricalCheckpoint>();
  for (const item of [...retained, ...(previous ? [previous] : []), checkpoint]) verified.set(checkpointId(item), item);
  qualifiedCheckpoints.set(clients, verified);
  return checkpoint;
}

const qualifications = new WeakMap<AgreedCheckpoint, RequalificationLink[]>();
const qualifiedCheckpoints = new WeakMap<EthereumEvidenceClients, Map<string, HistoricalCheckpoint>>();
const pendingRequalificationLinks = new WeakMap<EthereumEvidenceClients, Map<string, RetainedRequalificationLink>>();

export function receiptDate(seconds: string): Date {
  return new Date(Number(BigInt(timestamp(decimal(seconds))) * 1000n));
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
  clients: EthereumEvidenceClients, txHash: string, expected: { height: number; hash?: string; proofDigest?: string }, checkpoint: AgreedCheckpoint,
): Promise<AuthenticatedReceipt | undefined> {
  approved(clients, normalizedCheckpoint(checkpoint));
  if (checkpoint.schemaVersion !== 2) hold('legacy receipt authority requires requalification');
  const target = height(expected.height);
  const transactionHash = bytes(txHash, 32);
  const expectedHash = expected.hash === undefined ? undefined : bytes(expected.hash, 32);
  const expectedProof = expected.proofDigest === undefined ? undefined : bytes(expected.proofDigest, 32);
  if (target > BigInt(checkpoint.number)) return undefined;
  const response = await clients.native.request({ op: 'receipts', checkpoint: checkpoint.native,
    originals: [{ hash: transactionHash, number: target.toString(), ...(expectedHash ? { blockHash: expectedHash } : {}),
      ...(expectedProof ? { proofDigest: expectedProof } : {}) }] });
  exactObject(response, ['id', 'ok', 'op', 'checkpoint', 'receipts']);
  if (response.op !== 'receipts' || !isDeepStrictEqual(response.checkpoint, checkpoint.native) ||
      !Array.isArray(response.receipts) || response.receipts.length !== 1) hold('native original checkpoint changed');
  const item = exactObject(response.receipts[0], ['hash', 'number', 'blockHash', 'receipt', 'blockTimestamp', 'proofDigest']);
  const receipt = normalizeReceipt(item.receipt);
  if (item.hash !== transactionHash || item.number !== target.toString() || item.blockHash !== receipt.blockHash ||
      receipt.transactionHash !== transactionHash || BigInt(receipt.blockNumber) !== target ||
      (expectedHash !== undefined && receipt.blockHash !== expectedHash)) hold('original Ethereum receipt identity changed');
  if (expectedProof !== undefined && item.proofDigest !== expectedProof) hold('native original inclusion proof identity changed');
  return { receipt, blockTimestamp: timestamp(decimal(item.blockTimestamp)), checkpoint,
    inclusionProofDigest: bytes(item.proofDigest, 32), checkpointProofDigest: checkpoint.native.beacon.proofDigest,
    requalificationLinks: qualifications.get(checkpoint) ?? [] };
}

export function createReceiptAuthenticator(clients: EthereumEvidenceClients, checkpoint: AgreedCheckpoint): ReceiptAuthenticator {
  const receipts = new Map<string, Promise<AuthenticatedReceipt | undefined>>();
  return (hash, expected) => {
    const key = `${bytes(hash, 32)}:${height(expected.height)}:${expected.hash === undefined ? '' : bytes(expected.hash, 32)}:${expected.proofDigest === undefined ? '' : bytes(expected.proofDigest, 32)}`;
    let result = receipts.get(key);
    if (!result) {
      result = authenticateReceipt(clients, hash, expected, checkpoint);
      receipts.set(key, result);
    }
    return result;
  };
}

export function checkpointId(checkpoint: HistoricalCheckpoint): string {
  const value = normalizedCheckpoint(checkpoint);
  const legacy = `${value.chainId}:${value.genesisHash}:${value.number}`;
  return value.schemaVersion === 1 ? legacy : `${value.policy}:${legacy}:${value.native.beacon.proofDigest}`;
}

export async function loadCheckpoint(store: Store, clients: EthereumEvidenceClients): Promise<HistoricalCheckpoint | undefined> {
  const checkpoints = await loadCheckpoints(store, clients);
  return checkpoints.reduce<HistoricalCheckpoint | undefined>((highest, current) =>
    !highest || BigInt(current.number) > BigInt(highest.number) ? current : highest, undefined);
}

async function loadCheckpoints(store: Store, clients: EthereumEvidenceClients): Promise<HistoricalCheckpoint[]> {
  approved(clients);
  const rows = await store.find(EthereumFinalityCheckpoint, {});
  return rows.map((row) => {
    const checkpoint = normalizedCheckpoint(row.checkpoint);
    approved(clients, checkpoint);
    if (row.id !== checkpointId(checkpoint) || BigInt(row.blockNumber) !== BigInt(checkpoint.number)) hold('retained checkpoint row identity is inconsistent');
    return row.checkpoint;
  });
}

export async function qualifyStoredFinality(
  store: Store, clients: EthereumEvidenceClients, targets: readonly { number: string; hash: string }[] = [],
): Promise<AgreedCheckpoint> {
  pendingRequalificationLinks.set(clients, new Map());
  return qualifyFinality(clients, undefined, targets, await loadCheckpoints(store, clients));
}

export async function recheckFinality(clients: EthereumEvidenceClients, checkpoint: AgreedCheckpoint): Promise<void> {
  // A moving head must not replace the original page's receipt authorization references.
  const pending = pendingRequalificationLinks.get(clients);
  if (pending?.size) {
    const legacy = [...(qualifiedCheckpoints.get(clients)?.values() ?? [])].filter((item) => item.schemaVersion === 1);
    await qualifyFinality(clients, checkpoint, [], legacy, [...pending.values()]);
  }
  const response = await clients.native.request({ op: 'receipts', checkpoint: checkpoint.native, originals: [] });
  exactObject(response, ['id', 'ok', 'op', 'checkpoint', 'receipts']);
  if (response.op !== 'receipts' || !isDeepStrictEqual(response.checkpoint, checkpoint.native) ||
      !Array.isArray(response.receipts) || response.receipts.length) hold('fixed native checkpoint changed');
  pending?.clear();
}

export function proofReference(original: ProofReference): ProofReference {
  const inclusionProofDigest = bytes(original.inclusionProofDigest, 32);
  const checkpointProofDigest = bytes(original.checkpointProofDigest, 32);
  if (inclusionProofDigest !== original.inclusionProofDigest || checkpointProofDigest !== original.checkpointProofDigest) {
    hold('noncanonical receipt proof reference');
  }
  return { inclusionProofDigest, checkpointProofDigest, requalificationLinks: links(original.requalificationLinks) };
}

export function receiptAuthorization(
  original: AuthenticatedReceipt, requalificationLinks: RequalificationLink[] = [],
): Omit<SourceReceiptAuth, 'localLogIndex'> {
  return { version: 2, checkpointId: checkpointId(original.checkpoint), receipt: original.receipt,
    blockTimestamp: original.blockTimestamp, ...proofReference(original), requalificationLinks: links(requalificationLinks) };
}

export function requalificationLinksFor(
  clients: EthereumEvidenceClients, original: AuthenticatedReceipt,
  checkpointIds: readonly (string | undefined)[], retained?: readonly RequalificationLink[],
): RequalificationLink[] {
  const required = new Set<string>();
  for (const id of checkpointIds) {
    if (id === undefined) continue;
    const checkpoint = qualifiedCheckpoints.get(clients)?.get(id);
    if (!checkpoint) hold('missing archived authorization checkpoint');
    required.add(legacyCheckpointDigest(checkpoint));
  }
  const candidates = retained === undefined ? original.requalificationLinks : links(retained);
  const selected = candidates.filter((link) => required.has(link.originalDigest));
  if (selected.length !== required.size || (retained !== undefined && selected.length !== retained.length)) {
    hold('receipt authorization does not bind its exact archived checkpoints');
  }
  return selected;
}

export function assertAuthorizationCheckpoint(
  clients: EthereumEvidenceClients, id: string, block: bigint, proofDigest?: string,
): void {
  const checkpoint = qualifiedCheckpoints.get(clients)?.get(id);
  if (!checkpoint || BigInt(checkpoint.number) < block) hold('missing original authorization checkpoint fence');
  if (proofDigest !== undefined && (checkpoint.schemaVersion !== 2 || checkpoint.native.beacon.proofDigest !== proofDigest)) {
    hold('receipt authorization proof does not match its checkpoint');
  }
}

export function assertReceiptAuthorization(
  clients: EthereumEvidenceClients, retained: Omit<SourceReceiptAuth, 'localLogIndex'>, original: AuthenticatedReceipt,
): void {
  if (retained.version !== 2 || retained.blockTimestamp !== original.blockTimestamp ||
      !isDeepStrictEqual(retained.receipt, original.receipt)) hold('retained receipt authorization changed');
  proofReference(retained);
  assertAuthorizationCheckpoint(clients, retained.checkpointId, BigInt(original.receipt.blockNumber), retained.checkpointProofDigest);
  if (retained.inclusionProofDigest !== original.inclusionProofDigest) hold('retained original inclusion proof changed');
  if (retained.requalificationLinks.length) {
    const checkpoint = qualifiedCheckpoints.get(clients)?.get(retained.checkpointId);
    if (!checkpoint || checkpoint.schemaVersion !== 2) hold('missing original native link checkpoint');
    let pending = pendingRequalificationLinks.get(clients);
    if (!pending) { pending = new Map(); pendingRequalificationLinks.set(clients, pending); }
    for (const link of retained.requalificationLinks) {
      pending.set(`${link.originalDigest}:${link.proofDigest}:${checkpoint.native.beacon.proofDigest}`,
        { ...link, checkpoint: checkpoint.native });
    }
  }
}

export async function retainCheckpoint(store: Store, checkpoint: AgreedCheckpoint): Promise<void> {
  const normalized = normalizedCheckpoint(checkpoint);
  const id = checkpointId(normalized);
  const existing = await store.findOneBy(EthereumFinalityCheckpoint, { id });
  if (existing) {
    if (existing.id !== id || BigInt(existing.blockNumber) !== BigInt(normalized.number) ||
        !isDeepStrictEqual(normalizedCheckpoint(existing.checkpoint), normalized)) hold('conflicting retained Ethereum checkpoint');
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
  return qualifyStoredFinality(ctx.store, createEthereumEvidenceClients('eth'),
    headers.map((item) => ({ number: height(item.height).toString(), hash: bytes(item.hash, 32) })));
}
