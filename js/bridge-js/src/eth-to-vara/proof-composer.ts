import { ByteVectorType } from '@chainsafe/ssz';
import { MapDB, hexToBytes, bigIntToBytes, concatBytes, bytesToHex } from '@ethereumjs/util';
import { TransactionReceipt, TransactionType } from 'viem';
import { GearApi, HexString } from '@gear-js/api';
import { blake2AsHex } from '@polkadot/util-crypto';
import type { Bytes } from '@polkadot/types';
import { QueryBuilder } from 'sails-js';
import { encode as rlpEncode } from '@ethereumjs/rlp';
import { Trie } from '@ethereumjs/trie';
import { ssz } from '@lodestar/types';

import { BeaconClient, EthereumClient } from '../ethereum/index.js';
import {
  BlockGenericForBlockBody,
  BlockHeader,
  BlockInclusionProof,
  CheckpointClient,
  EthEventsClient,
  HistoricalProxyClient,
  ProofResult,
} from '../vara/index.js';
import { StatusCb, withOriginalDeadline } from '../util.js';
import { nativeWrapperIdlSha256, VFT_MANAGER_IDL_SHA256, type NativeWrapperMode } from './token-effect.js';

export type CheckpointNetwork = 'Mainnet' | 'Sepolia' | 'Holesky' | 'Hoodi' | Readonly<{
  localFixture: Readonly<{ genesis_time: bigint; genesis_validators_root: HexString }>;
}>;

export interface OwnedFixtureNetwork {
  schemaVersion: 1;
  ethereumChainId: 31337;
  ethereumGenesisHash: HexString;
  beaconGenesisValidatorsRoot: HexString;
  beaconGenesisTime: number;
}


export interface InboundProofProfile {
  readonly ethereumChainId: bigint;
  readonly ethereumGenesisHash: HexString;
  readonly beaconGenesisValidatorsRoot: HexString;
  readonly beaconGenesisTime: bigint;
  readonly sourceGenesisHash: HexString;
  readonly sourceBlockHash: HexString;
  /** Blake2-256 of raw source :code, not build-artifact SHA256. */
  readonly sourceRuntimeCodeHash: HexString;
  /** Independently requalified runtime activations; never learned from the connected RPC. */
  readonly runtimeTransitions?: readonly Readonly<{ blockHash: HexString; codeHash: HexString }>[];
  readonly nativeEth?: Readonly<{ ethereumToken: HexString; runtimeCodeHash: HexString; varaToken: HexString; varaCodeId: HexString }>;
  readonly queueDeployment?: Readonly<{ address: HexString; block: bigint }>;
  readonly historicalReceipts?: readonly Readonly<{ managerId: HexString; managerCodeId: HexString; endpointId: HexString; endpointCodeId: HexString; framing: 'network-rlp'; minimumSlot: bigint; maximumSlot: bigint }>[];
  readonly historicalProxyId: HexString;
  readonly historicalProxyCodeId: HexString;
  readonly historicalProxyIdlSha256: string;
  readonly endpoint: Readonly<{ programId: HexString; codeId: HexString; idlSha256: string; framing: 'electra' }>;
  readonly checkpoint: Readonly<{ programId: HexString; codeId: HexString; idlSha256: string; network: CheckpointNetwork }>;
  readonly consumer: Readonly<{ programId: HexString; codeId: HexString; idlSha256: string; service: string; method: string }>;
  readonly nativeWrapper?: Readonly<{ programId: HexString; ethereumToken: HexString; codeId: HexString; mode: NativeWrapperMode; idlSha256: string }>;
  readonly forks: readonly Readonly<{ name: string; epoch: bigint; version: HexString }>[];
}

/** Snapshot independently approved pins before I/O; never learn deployment identity from RPC. */
export function immutableInboundProfile(profile: InboundProofProfile): InboundProofProfile {
  if (!profile) throw new Error('HOLD: an authenticated inbound proof profile is required');
  let copy = structuredClone(profile);
  const hashes = [copy.ethereumGenesisHash, copy.beaconGenesisValidatorsRoot, copy.sourceGenesisHash, copy.sourceBlockHash,
    copy.sourceRuntimeCodeHash, copy.historicalProxyId, copy.historicalProxyCodeId, copy.endpoint?.programId, copy.endpoint?.codeId,
    copy.checkpoint?.programId, copy.checkpoint?.codeId, copy.consumer?.programId, copy.consumer?.codeId,
    ...(copy.nativeWrapper ? [copy.nativeWrapper.programId, copy.nativeWrapper.codeId] : [])];
  if (hashes.some(hash => typeof hash !== 'string' || !/^0x[0-9a-fA-F]{64}$/.test(hash)) ||
      typeof copy.ethereumChainId !== 'bigint' || copy.ethereumChainId <= 0n ||
      typeof copy.beaconGenesisTime !== 'bigint' || copy.beaconGenesisTime < 0n ||
      [copy.historicalProxyIdlSha256, copy.endpoint?.idlSha256, copy.checkpoint?.idlSha256, copy.consumer?.idlSha256,
        ...(copy.nativeWrapper ? [copy.nativeWrapper.idlSha256] : [])].some(hash => typeof hash !== 'string' || !/^[0-9a-fA-F]{64}$/.test(hash)) ||
      (copy.nativeWrapper && (typeof copy.nativeWrapper.ethereumToken !== 'string' || !/^0x[0-9a-fA-F]{40}$/.test(copy.nativeWrapper.ethereumToken) ||
        [copy.nativeWrapper.programId, copy.nativeWrapper.codeId, copy.nativeWrapper.ethereumToken].some(id => BigInt(id) === 0n))) ||
      typeof copy.consumer?.service !== 'string' || !copy.consumer.service || typeof copy.consumer?.method !== 'string' || !copy.consumer.method ||
      !Array.isArray(copy.forks) || !copy.forks.length || copy.forks.some(fork => typeof fork.name !== 'string' ||
        typeof fork.epoch !== 'bigint' || fork.epoch < 0n || !/^0x[0-9a-fA-F]{8}$/.test(fork.version)) ||
      (typeof copy.checkpoint.network === 'string' ? !['Mainnet', 'Sepolia', 'Holesky', 'Hoodi'].includes(copy.checkpoint.network) :
        !copy.checkpoint.network || Object.keys(copy.checkpoint.network).length !== 1 || !('localFixture' in copy.checkpoint.network))) {
    throw new Error('HOLD: incomplete or unknown inbound deployment profile');
  }
  const nonzeroHex = (value: unknown, bytes: number) => typeof value === 'string' && new RegExp(`^0x[0-9a-fA-F]{${bytes * 2}}$`).test(value) && BigInt(value) !== 0n;
  if ((copy.runtimeTransitions && (!Array.isArray(copy.runtimeTransitions) || copy.runtimeTransitions.some(row => !nonzeroHex(row.blockHash, 32) || !nonzeroHex(row.codeHash, 32)))) ||
      (copy.nativeEth && (!nonzeroHex(copy.nativeEth.ethereumToken, 20) || !nonzeroHex(copy.nativeEth.runtimeCodeHash, 32) || !nonzeroHex(copy.nativeEth.varaToken, 32) || !nonzeroHex(copy.nativeEth.varaCodeId, 32))) ||
      (copy.queueDeployment && (!nonzeroHex(copy.queueDeployment.address, 20) || typeof copy.queueDeployment.block !== 'bigint' || copy.queueDeployment.block < 0n)) ||
      (copy.historicalReceipts && (!Array.isArray(copy.historicalReceipts) || copy.historicalReceipts.some(row =>
        ![row.managerId, row.managerCodeId, row.endpointId, row.endpointCodeId].every(id => nonzeroHex(id, 32)) ||
        row.framing !== 'network-rlp' || typeof row.minimumSlot !== 'bigint' || typeof row.maximumSlot !== 'bigint' || row.minimumSlot < 0n || row.maximumSlot < row.minimumSlot)))) {
    throw new Error('HOLD: malformed independently approved capability');
  }
  for (const [index, row] of (copy.historicalReceipts ?? []).entries()) {
    if (copy.historicalReceipts!.slice(0, index).some(other => other.managerId.toLowerCase() === row.managerId.toLowerCase() && other.minimumSlot <= row.maximumSlot && row.minimumSlot <= other.maximumSlot)) throw new Error('HOLD: overlapping historical receipt approvals');
  }
  const lowerHex = (value: HexString) => value.toLowerCase() as HexString;
  const actor = <T extends { programId: HexString; codeId: HexString; idlSha256: string }>(value: T): T =>
    ({ ...value, programId: lowerHex(value.programId), codeId: lowerHex(value.codeId), idlSha256: value.idlSha256.toLowerCase() });
  copy = { ...copy, ethereumGenesisHash: lowerHex(copy.ethereumGenesisHash),
    beaconGenesisValidatorsRoot: lowerHex(copy.beaconGenesisValidatorsRoot), sourceGenesisHash: lowerHex(copy.sourceGenesisHash),
    sourceBlockHash: lowerHex(copy.sourceBlockHash), sourceRuntimeCodeHash: lowerHex(copy.sourceRuntimeCodeHash),
    historicalProxyId: lowerHex(copy.historicalProxyId), historicalProxyCodeId: lowerHex(copy.historicalProxyCodeId),
    historicalProxyIdlSha256: copy.historicalProxyIdlSha256.toLowerCase(),
    endpoint: actor(copy.endpoint), checkpoint: actor(copy.checkpoint), consumer: actor(copy.consumer),
    runtimeTransitions: copy.runtimeTransitions && Object.freeze(copy.runtimeTransitions.map(row => Object.freeze({ blockHash: lowerHex(row.blockHash), codeHash: lowerHex(row.codeHash) }))),
    nativeEth: copy.nativeEth && Object.freeze({ ethereumToken: lowerHex(copy.nativeEth.ethereumToken), runtimeCodeHash: lowerHex(copy.nativeEth.runtimeCodeHash), varaToken: lowerHex(copy.nativeEth.varaToken), varaCodeId: lowerHex(copy.nativeEth.varaCodeId) }),
    queueDeployment: copy.queueDeployment && Object.freeze({ ...copy.queueDeployment, address: lowerHex(copy.queueDeployment.address) }),
    historicalReceipts: copy.historicalReceipts && Object.freeze(copy.historicalReceipts.map(row => Object.freeze({ ...row, managerId: lowerHex(row.managerId), managerCodeId: lowerHex(row.managerCodeId), endpointId: lowerHex(row.endpointId), endpointCodeId: lowerHex(row.endpointCodeId) }))),
    nativeWrapper: copy.nativeWrapper ? { ...actor(copy.nativeWrapper), ethereumToken: lowerHex(copy.nativeWrapper.ethereumToken) } : undefined,
    forks: copy.forks.map(fork => ({ ...fork, version: lowerHex(fork.version) })) };
  if (copy.endpoint.framing !== 'electra' || copy.endpoint.idlSha256 !== ELECTRA_IDL_SHA256 ||
      (copy.checkpoint.idlSha256 !== CHECKPOINT_IDL_SHA256 &&
        !(typeof copy.checkpoint.network === 'string' && copy.checkpoint.idlSha256 === LEGACY_PUBLIC_CHECKPOINT_IDL_SHA256)) ||
      copy.historicalProxyIdlSha256 !== PROXY_IDL_SHA256 ||
      (copy.consumer.service === 'VftManager' && copy.consumer.idlSha256 !== VFT_MANAGER_IDL_SHA256 &&
        !copy.historicalReceipts?.some(row => row.managerId === copy.consumer.programId && row.managerCodeId === copy.consumer.codeId && row.endpointId === copy.endpoint.programId && row.endpointCodeId === copy.endpoint.codeId)) ||
      (copy.nativeWrapper && copy.nativeWrapper.idlSha256 !== nativeWrapperIdlSha256(copy.nativeWrapper.mode))) {
    throw new Error('HOLD: unqualified deployed inbound IDL or proof framing');
  }
  if (typeof copy.checkpoint.network !== 'string') {
    const fixture = copy.checkpoint.network.localFixture;
    const names = ['phase0', 'altair', 'bellatrix', 'capella', 'deneb', 'electra', 'fulu'];
    if (!fixture || Object.keys(fixture).length !== 2 || typeof fixture.genesis_time !== 'bigint' ||
        fixture.genesis_time !== copy.beaconGenesisTime || fixture.genesis_time > BigInt(Number.MAX_SAFE_INTEGER) ||
        typeof fixture.genesis_validators_root !== 'string' || fixture.genesis_validators_root.toLowerCase() !== copy.beaconGenesisValidatorsRoot.toLowerCase() ||
        copy.ethereumChainId !== 31337n || copy.forks.length !== names.length ||
        copy.forks.some((fork, index) => fork.name !== names[index] || fork.epoch !== (fork.name === 'fulu' ? 18446744073709551615n : 0n)) ||
        copy.forks[5].version.toLowerCase() !== '0x60003137') {
      throw new Error('HOLD: LocalFixture identity or fixed Electra fork schedule differs from approval');
    }
    copy = { ...copy, checkpoint: { ...copy.checkpoint,
      network: { localFixture: { ...fixture, genesis_validators_root: lowerHex(fixture.genesis_validators_root) } } } };
    if (typeof copy.checkpoint.network !== 'string') Object.freeze(copy.checkpoint.network.localFixture);
    Object.freeze(copy.checkpoint.network);
  }
  Object.freeze(copy.endpoint); Object.freeze(copy.checkpoint); Object.freeze(copy.consumer);
  if (copy.nativeWrapper) Object.freeze(copy.nativeWrapper);
  copy.forks.forEach(Object.freeze); Object.freeze(copy.forks);
  return Object.freeze(copy);
}

/** Authenticate the original bootstrap and a finalized runtime activation without replacing either pin. */
export async function authenticateInboundSource(api: GearApi, supplied: InboundProofProfile, pin: HexString): Promise<void> {
  const profile = immutableInboundProfile(supplied);
  if (api.genesisHash.toHex().toLowerCase() !== profile.sourceGenesisHash) throw new Error('HOLD: source genesis mismatch');
  const [header, anchor, finalizedHash] = await Promise.all([api.rpc.chain.getHeader(pin), api.rpc.chain.getHeader(profile.sourceBlockHash), api.blocks.getFinalizedHead()]);
  const finalized = await api.rpc.chain.getHeader(finalizedHash);
  if (header.number.toBigInt() > finalized.number.toBigInt() || header.number.toBigInt() < anchor.number.toBigInt() ||
      !(await api.blocks.getBlockHash(header.number.toBigInt())).eq(pin) || !(await api.blocks.getBlockHash(anchor.number.toBigInt())).eq(profile.sourceBlockHash)) {
    throw new Error('HOLD: source pin is not canonical finalized approved history');
  }
  const anchorCode = await api.rpc.state.getStorage<Bytes>(':code', profile.sourceBlockHash);
  if (!anchorCode || anchorCode.isEmpty || blake2AsHex(anchorCode.toU8a(true), 256) !== profile.sourceRuntimeCodeHash) throw new Error('HOLD: original runtime approval changed');
  let expected = profile.sourceRuntimeCodeHash;
  let activation = anchor.number.toBigInt();
  for (const transition of profile.runtimeTransitions ?? []) {
    const at = await api.rpc.chain.getHeader(transition.blockHash);
    if (at.number.toBigInt() <= activation) throw new Error('HOLD: runtime approvals must be ordered after bootstrap');
    activation = at.number.toBigInt();
    if (activation > header.number.toBigInt()) continue;
    const code = await api.rpc.state.getStorage<Bytes>(':code', transition.blockHash);
    if (!(await api.blocks.getBlockHash(activation)).eq(transition.blockHash) || !code || code.isEmpty || blake2AsHex(code.toU8a(true), 256) !== transition.codeHash) throw new Error('HOLD: runtime activation differs from approval');
    expected = transition.codeHash;
  }
  const code = await api.rpc.state.getStorage<Bytes>(':code', pin);
  if (!code || code.isEmpty || blake2AsHex(code.toU8a(true), 256) !== expected) throw new Error('HOLD: unapproved runtime at source pin');
}

/** Parse caller-approved JSON, including the nested Sails fixture identity. */
export function parseInboundProofProfile(value: string): InboundProofProfile {
  const supplied = JSON.parse(value);
  const network = supplied.checkpoint?.network;
  return immutableInboundProfile({ ...supplied, ethereumChainId: BigInt(supplied.ethereumChainId),
    beaconGenesisTime: BigInt(supplied.beaconGenesisTime),
    queueDeployment: supplied.queueDeployment && { ...supplied.queueDeployment, block: BigInt(supplied.queueDeployment.block) },
    historicalReceipts: supplied.historicalReceipts?.map((row: NonNullable<InboundProofProfile['historicalReceipts']>[number]) => ({ ...row, minimumSlot: BigInt(row.minimumSlot), maximumSlot: BigInt(row.maximumSlot) })),
    checkpoint: { ...supplied.checkpoint, network: typeof network === 'string' ? network :
      { ...network, localFixture: { ...network?.localFixture, genesis_time: BigInt(network?.localFixture?.genesis_time) } } },
    forks: supplied.forks.map((fork: { epoch: string }) => ({ ...fork, epoch: BigInt(fork.epoch) })) });
}

/** Public enums decode as strings (old IDL) or null-valued objects (new IDL). */
export function checkpointNetworkMatches(observed: unknown, approved: CheckpointNetwork): boolean {
  if (typeof approved === 'string') {
    return observed === approved || (!!observed && typeof observed === 'object' && Object.keys(observed).length === 1 &&
      (observed as Record<string, unknown>)[approved.toLowerCase()] === null);
  }
  if (!observed || typeof observed !== 'object' || Object.keys(observed).length !== 1 || !('localFixture' in observed)) return false;
  const fixture = observed.localFixture as { genesis_time?: unknown; genesis_validators_root?: unknown } | null;
  if (!fixture || Object.keys(fixture).length !== 2 ||
      !['number', 'string', 'bigint'].includes(typeof fixture.genesis_time) ||
      (typeof fixture.genesis_time === 'number' && !Number.isSafeInteger(fixture.genesis_time)) ||
      typeof fixture.genesis_validators_root !== 'string') return false;
  try {
    return BigInt(fixture.genesis_time as string | number | bigint) === approved.localFixture.genesis_time &&
      fixture.genesis_validators_root.toLowerCase() === approved.localFixture.genesis_validators_root.toLowerCase();
  } catch { return false; }
}

/** Validate offline chain approval without requiring an inbound consumer profile. */
export function immutableOwnedFixtureNetwork(descriptor: OwnedFixtureNetwork, transports: readonly string[]): Readonly<OwnedFixtureNetwork> {
  if (!descriptor || Object.keys(descriptor).sort().join(',') !== 'beaconGenesisTime,beaconGenesisValidatorsRoot,ethereumChainId,ethereumGenesisHash,schemaVersion' ||
      descriptor.schemaVersion !== 1 || descriptor.ethereumChainId !== 31337 ||
      [descriptor.ethereumGenesisHash, descriptor.beaconGenesisValidatorsRoot].some(hash => typeof hash !== 'string' ||
        !/^0x[0-9a-fA-F]{64}$/.test(hash) || BigInt(hash) === 0n) ||
      !Number.isSafeInteger(descriptor.beaconGenesisTime) || descriptor.beaconGenesisTime <= 0 || !transports.length) {
    throw new Error('HOLD: independently supplied owned-fixture-network is malformed');
  }
  for (const transport of transports) {
    const url = new URL(transport);
    if (!['http:', 'https:', 'ws:', 'wss:'].includes(url.protocol) ||
        !['127.0.0.1', '[::1]'].includes(url.hostname) || url.username || url.password) {
      throw new Error('HOLD: LocalFixture transports must use numeric loopback addresses');
    }
  }
  return Object.freeze({ ...structuredClone(descriptor),
    ethereumGenesisHash: descriptor.ethereumGenesisHash.toLowerCase() as HexString,
    beaconGenesisValidatorsRoot: descriptor.beaconGenesisValidatorsRoot.toLowerCase() as HexString });
}

/** Offline profile and chain approval must match; live RPC is never approval. */
export function validateOwnedFixtureNetwork(profile: InboundProofProfile, descriptor: OwnedFixtureNetwork, transports: readonly string[]): void {
  profile = immutableInboundProfile(profile);
  descriptor = immutableOwnedFixtureNetwork(descriptor, transports);
  if (typeof profile.checkpoint.network === 'string' || profile.ethereumChainId !== BigInt(descriptor.ethereumChainId) ||
      descriptor.ethereumGenesisHash.toLowerCase() !== profile.ethereumGenesisHash.toLowerCase() ||
      descriptor.beaconGenesisValidatorsRoot.toLowerCase() !== profile.beaconGenesisValidatorsRoot.toLowerCase() ||
      BigInt(descriptor.beaconGenesisTime) !== profile.beaconGenesisTime) {
    throw new Error('HOLD: independently supplied owned-fixture-network differs from approved profile');
  }
}

export function validateLocalFixtureBeaconSpec(profile: InboundProofProfile, spec: Record<string, string>): void {
  if (typeof profile.checkpoint.network === 'string') return;
  if (spec.PRESET_BASE !== 'mainnet' || spec.SECONDS_PER_SLOT !== '12' || spec.SLOTS_PER_EPOCH !== '32' ||
      spec.EPOCHS_PER_SYNC_COMMITTEE_PERIOD !== '256' || spec.SYNC_COMMITTEE_SIZE !== '512' ||
      spec.DENEB_FORK_EPOCH !== '0' || spec.ELECTRA_FORK_EPOCH !== '0' ||
      spec.ELECTRA_FORK_VERSION?.toLowerCase() !== '0x60003137' || spec.FULU_FORK_EPOCH !== '18446744073709551615') {
    throw new Error('HOLD: LocalFixture requires the standard mainnet preset and fixed Electra-at-genesis schedule');
  }
}


const ELECTRA_IDL_SHA256 = '70cc917357c2a589063f1efca80b2da0390f3c12c58eecb0b9367228c4bce1b0';
const CHECKPOINT_IDL_SHA256 = '65c15d4eccb2a19a8ad240b82738a6335b0e9efd9a054340856f665addaca7e5';
const LEGACY_PUBLIC_CHECKPOINT_IDL_SHA256 = '2e5d5576576bdbbf8bfe7c3a9f83f48be400e602bf47d906d5ab3a7ff1f28b4f';
const PROXY_IDL_SHA256 = '4bde230e4759abfedd44070790b3a579d7f75aadff14266e0f1a287f61942bc7';

const BytesFixed96 = new ByteVectorType(96);
const BytesFixed256 = new ByteVectorType(256);

function txTypeToBytes(txType: TransactionType): Uint8Array {
  switch (txType) {
    case 'legacy':
      return new Uint8Array();
    case 'eip2930':
      return Uint8Array.of(0x01);
    case 'eip1559':
      return Uint8Array.of(0x02);
    case 'eip4844':
      return Uint8Array.of(0x03);
    case 'eip7702':
      return Uint8Array.of(0x04);
    default: {
      throw new Error(`Unknown tx type: ${txType}`);
    }
  }
}

export async function composeProof(
  beaconClient: BeaconClient,
  ethClient: EthereumClient,
  historicalProxyClient: HistoricalProxyClient,
  txHash: `0x${string}`,
  profile: InboundProofProfile,
  wait = false,
  statusCb: StatusCb = () => {},
  deadline = Date.now() + 44 * 60 * 1000,
): Promise<ProofResult> {
  profile = immutableInboundProfile(profile);
  const read = <T>(operation: () => Promise<T>) => withOriginalDeadline(deadline, operation);
  const api = historicalProxyClient.api;
  if (!api.genesisHash.eq(profile.sourceGenesisHash) ||
      historicalProxyClient.programId.toLowerCase() !== profile.historicalProxyId.toLowerCase() ||
      BigInt(beaconClient.genesisBlock.genesis_time) !== profile.beaconGenesisTime ||
      beaconClient.genesisBlock.genesis_validators_root.toLowerCase() !== profile.beaconGenesisValidatorsRoot.toLowerCase()) {
    throw new Error('Inbound proof source or Beacon network mismatch');
  }
  const sourceHeader = await read(() => api.rpc.chain.getHeader(profile.sourceBlockHash));
  const originalHeight = sourceHeader.number.toBigInt();
  let observedHash = profile.sourceBlockHash;
  let observedHeight = originalHeight;
  let previousHash = observedHash;
  let previousHeight = observedHeight;
  const authenticateSource = async (pin: HexString) => {
    const header = pin === profile.sourceBlockHash ? sourceHeader : await read(() => api.rpc.chain.getHeader(pin));
    const height = header.number.toBigInt();
    const finalizedHash = (await read(() => api.blocks.getFinalizedHead())).toHex();
    const finalizedHeader = await read(() => api.rpc.chain.getHeader(finalizedHash));
    if (height < observedHeight) throw new Error('Finalized inbound source history regressed');
    if (height > finalizedHeader.number.toBigInt()) throw new Error('Inbound preparation source pin is not canonical finalized history');
    const originalCanonical = await read(() => api.blocks.getBlockHash(originalHeight));
    const currentCanonical = height === originalHeight ? originalCanonical : await read(() => api.blocks.getBlockHash(height));
    const observedCanonical = observedHeight === originalHeight ? originalCanonical : observedHeight === height ? currentCanonical :
      await read(() => api.blocks.getBlockHash(observedHeight));
    const previousCanonical = previousHeight === originalHeight ? originalCanonical : previousHeight === observedHeight ? observedCanonical :
      await read(() => api.blocks.getBlockHash(previousHeight));
    if (!originalCanonical.eq(profile.sourceBlockHash) || !currentCanonical.eq(pin) || !observedCanonical.eq(observedHash) ||
        !previousCanonical.eq(previousHash)) {
      throw new Error('Original or observed finalized inbound source history changed');
    }
    await read(() => authenticateInboundSource(api, profile, pin));
    if (pin !== observedHash) { previousHash = observedHash; previousHeight = observedHeight; }
    observedHash = pin;
    observedHeight = height;
  };
  await authenticateSource(profile.sourceBlockHash);
  statusCb('Requesting transaction receipt', { txHash });
  const receipt = await read(() => ethClient.getTransactionReceipt(txHash));
  const block = await read(() => ethClient.getBlockByHash(receipt.blockHash));
  if (receipt.root !== undefined || receipt.status !== 'success' || receipt.transactionHash.toLowerCase() !== txHash.toLowerCase() ||
      block.hash !== receipt.blockHash || block.number !== receipt.blockNumber ||
      block.transactions[receipt.transactionIndex] !== receipt.transactionHash) {
    throw new Error('Original Ethereum receipt does not match its successful canonical block transaction');
  }
  const receipts = await read(() => Promise.all(block.transactions.map((hash) => ethClient.getTransactionReceipt(hash))));
  if (!receipts.every((item, index) => item.transactionIndex === index && item.blockHash === block.hash &&
      item.blockNumber === block.number && item.transactionHash === block.transactions[index])) {
    throw new Error('Incomplete or inconsistent original block receipts');
  }
  const slot = await read(() => ethClient.getSlot(block.number));
  if (!Number.isSafeInteger(slot) || slot < 0) throw new Error('Invalid original Ethereum receipt slot');
  const { proof, receiptRlp, root } = await read(() => generateMerkleProof(receipt.transactionIndex, receipts));
  if (bytesToHex(root).toLowerCase() !== block.receiptsRoot.toLowerCase()) {
    throw new Error('Generated receipt trie does not match the authenticated execution block receipts root');
  }

  const ethEvents = new EthEventsClient(api, profile.endpoint.programId);
  const checkpoint = new CheckpointClient(api, profile.checkpoint.programId);
  const bindings = [
    [historicalProxyClient.programId, profile.historicalProxyCodeId],
    [profile.endpoint.programId, profile.endpoint.codeId],
    [profile.checkpoint.programId, profile.checkpoint.codeId],
    [profile.consumer.programId, profile.consumer.codeId],
    ...(profile.nativeWrapper ? [[profile.nativeWrapper.programId, profile.nativeWrapper.codeId] as const] : []),
  ] as const;
  const authenticateBindings = async (pin: HexString) => {
    if (!checkpointNetworkMatches(await read(() => checkpoint.serviceState.network().atBlock(pin).call()), profile.checkpoint.network)) {
      throw new Error('Checkpoint immutable network differs from approved inbound profile');
    }
    const endpoint = await read(() => historicalProxyClient.historicalProxy.endpointFor(slot).atBlock(pin).call());
    if ('err' in endpoint || endpoint.ok.toLowerCase() !== profile.endpoint.programId.toLowerCase()) {
      throw new Error('Historical endpointFor(slot) differs from the authenticated deployed profile');
    }
    const checkpointId = await read(() => ethEvents.ethereumEventClient.checkpointLightClientAddress().atBlock(pin).call());
    if (checkpointId.toLowerCase() !== profile.checkpoint.programId.toLowerCase()) {
      throw new Error('Historical endpoint checkpoint identity mismatch');
    }
    for (const [id, expectedCodeId] of bindings) {
      const program = await read(() => api.programStorage.getProgram(id, pin));
      if (!program.state.isInitialized || !program.codeId.eq(expectedCodeId)) {
        throw new Error('Uninitialized or changed deployed inbound program CodeId: ' + id);
      }
    }
  };
  await authenticateBindings(observedHash);
  let appliedCheckpoint: [number | string | bigint, HexString];
  for (;;) {
    const checkpointResult = await read(() => new QueryBuilder<{ ok: [number | string | bigint, HexString] } | { err: string }>(
      api, checkpoint.registry, profile.checkpoint.programId, 'ServiceCheckpointFor', 'Get', slot, 'u64', 'Result<(u64, H256), CheckpointError>',
    ).atBlock(observedHash).call());
    if ('ok' in checkpointResult) { appliedCheckpoint = checkpointResult.ok; break; }
    if (!wait || checkpointResult.err !== 'NotPresent') {
      throw new Error('Checkpoint is not applied at the finalized preparation pin: ' + checkpointResult.err);
    }
    statusCb('Waiting for an applied finalized checkpoint', { slot: slot.toString(), sourceBlockHash: observedHash });
    await read(() => new Promise<void>(resolve => setTimeout(resolve, Math.min(3000, Math.max(0, deadline - Date.now())))));
    const nextPin = (await read(() => api.blocks.getFinalizedHead())).toHex();
    await authenticateSource(nextPin);
    await authenticateBindings(nextPin);
  }
  const checkpointSlot = Number(appliedCheckpoint[0]);
  if (!Number.isSafeInteger(checkpointSlot) || checkpointSlot < slot) throw new Error('Invalid applied checkpoint slot');
  const proofBlock = await buildInclusionProof(beaconClient, slot, [checkpointSlot, appliedCheckpoint[1]], profile, statusCb, deadline);
  const payload = proofBlock.block.body.executionPayload;
  if (bytesToHex(payload.blockHash).toLowerCase() !== receipt.blockHash.toLowerCase() ||
      BigInt(payload.blockNumber) !== receipt.blockNumber || bytesToHex(payload.receiptsRoot).toLowerCase() !== block.receiptsRoot.toLowerCase()) {
    throw new Error('Beacon execution payload does not authenticate the original execution receipt block');
  }
  await authenticateSource(observedHash);
  await authenticateBindings(observedHash);
  await authenticateSource(observedHash);
  return { proofBlock, proof, transactionIndex: receipt.transactionIndex, receiptRlp };
}

async function buildInclusionProof(
  beaconClient: BeaconClient,
  slot: number,
  checkpoint: [number, HexString],
  profile: InboundProofProfile,
  statusCb: StatusCb,
  deadline: number,
): Promise<BlockInclusionProof> {
  const read = <T>(operation: () => Promise<T>) => withOriginalDeadline(deadline, operation);
  const beaconBlock = await read(() => beaconClient.getBlock(slot));
  const spec = await read(() => beaconClient.getSpec());
  validateLocalFixtureBeaconSpec(profile, spec);
  const epoch = BigInt(slot) / 32n;
  let selected: InboundProofProfile['forks'][number] | undefined;
  let previousEpoch = -1n;
  let previousOrder = -1;
  for (const fork of profile.forks) {
    const prefix = fork.name.toUpperCase();
    const order = ['phase0', 'altair', 'bellatrix', 'capella', 'deneb', 'electra', 'fulu'].indexOf(fork.name);
    const versionKey = fork.name === 'phase0' ? 'GENESIS_FORK_VERSION' : prefix + '_FORK_VERSION';
    const observedEpoch = fork.name === 'phase0' ? 0n : BigInt(spec[prefix + '_FORK_EPOCH']);
    if (order <= previousOrder || fork.epoch < previousEpoch || fork.epoch !== observedEpoch || spec[versionKey]?.toLowerCase() !== fork.version.toLowerCase()) {
      throw new Error('Beacon fork schedule differs from the authenticated profile');
    }
    previousEpoch = fork.epoch;
    previousOrder = order;
    if (fork.epoch <= epoch) selected = fork;
  }
  for (const [key, value] of Object.entries(spec)) {
    if (key.endsWith('_FORK_EPOCH') && BigInt(value) <= epoch &&
        !profile.forks.some((fork) => fork.name.toUpperCase() + '_FORK_EPOCH' === key)) {
      throw new Error('Unreviewed active Beacon fork');
    }
  }
  if (!selected || selected.name !== beaconBlock.fork || (selected.name !== 'electra' && selected.name !== 'fulu')) {
    throw new Error('Unsupported or mismatched actual receipt Beacon fork');
  }
  const types = selected.name === 'fulu' ? ssz.fulu : ssz.electra;
  const body = types.BeaconBlockBody.fromJson(beaconBlock.body);
  const bodyTypes = types.BeaconBlockBody.fields;
  const selectedHeader = await read(() => beaconClient.getBlockHeader(slot));
  const blockRoot = bytesToHex(types.BeaconBlock.hashTreeRoot(types.BeaconBlock.fromJson(beaconBlock)));
  if (!selectedHeader.canonical || selectedHeader.root.toLowerCase() !== blockRoot.toLowerCase() ||
      Number(beaconBlock.slot) !== slot) throw new Error('Original Beacon block does not match its canonical header');
  const block: BlockGenericForBlockBody = {
    slot,
    proposerIndex: BigInt(beaconBlock.proposer_index),
    parentRoot: hexToBytes(beaconBlock.parent_root),
    stateRoot: hexToBytes(beaconBlock.state_root),
    body: {
      randaoReveal: BytesFixed96.hashTreeRoot(body.randaoReveal),
      eth1Data: bodyTypes.eth1Data.hashTreeRoot(body.eth1Data),
      graffiti: body.graffiti,
      proposerSlashings: bodyTypes.proposerSlashings.hashTreeRoot(body.proposerSlashings),
      attesterSlashings: bodyTypes.attesterSlashings.hashTreeRoot(body.attesterSlashings),
      attestations: bodyTypes.attestations.hashTreeRoot(body.attestations),
      deposits: bodyTypes.deposits.hashTreeRoot(body.deposits),
      voluntaryExits: bodyTypes.voluntaryExits.hashTreeRoot(body.voluntaryExits),
      syncAggregate: bodyTypes.syncAggregate.hashTreeRoot(body.syncAggregate),
      executionPayload: {
        ...body.executionPayload,
        logsBloom: BytesFixed256.hashTreeRoot(body.executionPayload.logsBloom),
        transactions: types.Transactions.hashTreeRoot(body.executionPayload.transactions),
        withdrawals: types.Withdrawals.hashTreeRoot(body.executionPayload.withdrawals),
      },
      blsToExecutionChanges: bodyTypes.blsToExecutionChanges.hashTreeRoot(body.blsToExecutionChanges),
      blobKzgCommitments: bodyTypes.blobKzgCommitments.hashTreeRoot(body.blobKzgCommitments),
      executionRequests: bodyTypes.executionRequests.hashTreeRoot(body.executionRequests),
    },
  };
  const [checkpointSlot, checkpointRoot] = checkpoint;
  if (checkpointSlot === slot) {
    if (blockRoot.toLowerCase() !== checkpointRoot.toLowerCase()) throw new Error('Applied checkpoint root differs from original block');
    return { block, headers: [] };
  }
  statusCb('Requesting canonical historical Beacon headers', { from: (slot + 1).toString(), to: checkpointSlot.toString() });
  const beaconHeaders = await read(() => beaconClient.requestHeaders(slot + 1, checkpointSlot));
  const headers: BlockHeader[] = [];
  let previousSlot = slot;
  let previousRoot = blockRoot;
  for (const header of beaconHeaders) {
    const message = header.header.message;
    const height = Number(message.slot);
    const root = bytesToHex(ssz.phase0.BeaconBlockHeader.hashTreeRoot(ssz.phase0.BeaconBlockHeader.fromJson(message)));
    if (!header.canonical || !Number.isSafeInteger(height) || height <= previousSlot || height > checkpointSlot ||
        message.parent_root.toLowerCase() !== previousRoot.toLowerCase() || root.toLowerCase() !== header.root.toLowerCase()) {
      throw new Error('Gap or conflicting canonical historical Beacon headers');
    }
    headers.push({ slot: height, proposerIndex: BigInt(message.proposer_index), parentRoot: hexToBytes(message.parent_root),
      stateRoot: hexToBytes(message.state_root), bodyRoot: hexToBytes(message.body_root) });
    previousSlot = height;
    previousRoot = root;
  }
  if (previousSlot !== checkpointSlot || previousRoot.toLowerCase() !== checkpointRoot.toLowerCase()) {
    throw new Error('Historical Beacon archive does not reach the original applied checkpoint');
  }
  return { block, headers };
}

function rlpEncodeReceipt(receipt: TransactionReceipt): Uint8Array {
  // https://eips.ethereum.org/EIPS/eip-2718#receipts
  if (receipt.root !== undefined && !/^0x[0-9a-fA-F]{64}$/.test(receipt.root)) {
    throw new Error('Invalid historical receipt post-state root');
  }
  if (receipt.root === undefined && receipt.status !== 'success' && receipt.status !== 'reverted') {
    throw new Error('Missing receipt execution status');
  }
  const status = receipt.root !== undefined ? hexToBytes(receipt.root) :
    receipt.status === 'success' ? Uint8Array.from([1]) : Uint8Array.from([]);
  const cumulativeGasUsed = receipt.cumulativeGasUsed === 0n ? new Uint8Array() : bigIntToBytes(receipt.cumulativeGasUsed);
  const bloom = hexToBytes(receipt.logsBloom);

  const logs = receipt.logs.map((log) => {
    const address = hexToBytes(log.address);
    const data = hexToBytes(log.data);
    return [address, log.topics.map((topic) => hexToBytes(topic)), data];
  });

  const txType = txTypeToBytes(receipt.type);
  const data = [status, cumulativeGasUsed, bloom, logs];
  const innerReceipt = rlpEncode(data);

  return concatBytes(txType, innerReceipt);
}

const rlpEncodeTransactionIndex = (index: number): Uint8Array => rlpEncode(index);

const rlpEncodeIndexAndReceipt = (
  index: number,
  receipt: TransactionReceipt,
): [encodedIndex: Uint8Array, encodedReceipt: Uint8Array] => [
  rlpEncodeTransactionIndex(index),
  rlpEncodeReceipt(receipt),
];

export async function generateMerkleProof(txIndex: number, receipts: TransactionReceipt[]) {
  const targetReceipt = receipts.find((receipt) => receipt.transactionIndex === txIndex);

  if (!targetReceipt) {
    throw new Error(`Transaction receipt not found for index ${txIndex}`);
  }

  const trie = await Trie.create({
    db: new MapDB(),
  });

  for (const receipt of receipts) {
    const [encodedIndex, encodedReceipt] = rlpEncodeIndexAndReceipt(receipt.transactionIndex, receipt);
    await trie.put(encodedIndex, encodedReceipt);
  }

  const targetEncodedIndex = rlpEncodeTransactionIndex(txIndex);

  const [proof, receipt] = await Promise.all([trie.createProof(targetEncodedIndex), trie.get(targetEncodedIndex)]);

  if (!receipt) {
    throw new Error(`Value not found for index ${txIndex}`);
  }

  return {
    proof,
    receiptRlp: receipt,
    root: trie.root(),
  };
}
