import { ByteVectorType } from '@chainsafe/ssz';
import { MapDB, hexToBytes, bigIntToBytes, concatBytes, bytesToHex } from '@ethereumjs/util';
import { TransactionReceipt, TransactionType } from 'viem';
import { HexString } from '@gear-js/api';
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
import { NATIVE_WRAPPER_IDL_SHA256 } from './token-effect.js';

export interface InboundProofProfile {
  readonly ethereumChainId: bigint;
  readonly ethereumGenesisHash: HexString;
  readonly beaconGenesisValidatorsRoot: HexString;
  readonly beaconGenesisTime: bigint;
  readonly sourceGenesisHash: HexString;
  readonly sourceBlockHash: HexString;
  /** Blake2-256 of raw source :code, not build-artifact SHA256. */
  readonly sourceRuntimeCodeHash: HexString;
  readonly historicalProxyId: HexString;
  readonly historicalProxyCodeId: HexString;
  readonly historicalProxyIdlSha256: string;
  readonly endpoint: Readonly<{ programId: HexString; codeId: HexString; idlSha256: string; framing: 'electra' }>;
  readonly checkpoint: Readonly<{ programId: HexString; codeId: HexString; idlSha256: string; network: 'Mainnet' | 'Sepolia' | 'Holesky' | 'Hoodi' }>;
  readonly consumer: Readonly<{ programId: HexString; codeId: HexString; idlSha256: string; service: string; method: string }>;
  readonly nativeWrapper?: Readonly<{ programId: HexString; codeId: HexString; idlSha256: string }>;
  readonly forks: readonly Readonly<{ name: string; epoch: bigint; version: HexString }>[];
}

/** Snapshot independently approved pins before I/O; never learn deployment identity from RPC. */
export function immutableInboundProfile(profile: InboundProofProfile): InboundProofProfile {
  if (!profile) throw new Error('HOLD: an authenticated inbound proof profile is required');
  const copy = structuredClone(profile);
  const hashes = [copy.ethereumGenesisHash, copy.beaconGenesisValidatorsRoot, copy.sourceGenesisHash, copy.sourceBlockHash,
    copy.sourceRuntimeCodeHash, copy.historicalProxyId, copy.historicalProxyCodeId, copy.endpoint?.programId, copy.endpoint?.codeId,
    copy.checkpoint?.programId, copy.checkpoint?.codeId, copy.consumer?.programId, copy.consumer?.codeId,
    ...(copy.nativeWrapper ? [copy.nativeWrapper.programId, copy.nativeWrapper.codeId] : [])];
  if (hashes.some(hash => typeof hash !== 'string' || !/^0x[0-9a-fA-F]{64}$/.test(hash)) ||
      typeof copy.ethereumChainId !== 'bigint' || copy.ethereumChainId <= 0n ||
      typeof copy.beaconGenesisTime !== 'bigint' || copy.beaconGenesisTime < 0n ||
      [copy.historicalProxyIdlSha256, copy.endpoint?.idlSha256, copy.checkpoint?.idlSha256, copy.consumer?.idlSha256].some(hash => !/^[0-9a-f]{64}$/.test(hash ?? '')) ||
      typeof copy.consumer?.service !== 'string' || !copy.consumer.service || typeof copy.consumer?.method !== 'string' || !copy.consumer.method ||
      !Array.isArray(copy.forks) || !copy.forks.length || copy.forks.some(fork => typeof fork.name !== 'string' ||
        typeof fork.epoch !== 'bigint' || fork.epoch < 0n || !/^0x[0-9a-fA-F]{8}$/.test(fork.version)) ||
      !['Mainnet', 'Sepolia', 'Holesky', 'Hoodi'].includes(copy.checkpoint.network)) {
    throw new Error('HOLD: incomplete or unknown inbound deployment profile');
  }
  if (copy.endpoint.framing !== 'electra' || copy.endpoint.idlSha256 !== ELECTRA_IDL_SHA256 ||
      copy.checkpoint.idlSha256 !== CHECKPOINT_IDL_SHA256 || copy.historicalProxyIdlSha256 !== PROXY_IDL_SHA256 ||
      (copy.nativeWrapper && copy.nativeWrapper.idlSha256 !== NATIVE_WRAPPER_IDL_SHA256)) {
    throw new Error('HOLD: unqualified deployed inbound IDL or proof framing');
  }
  Object.freeze(copy.endpoint); Object.freeze(copy.checkpoint); Object.freeze(copy.consumer);
  if (copy.nativeWrapper) Object.freeze(copy.nativeWrapper);
  copy.forks.forEach(Object.freeze); Object.freeze(copy.forks);
  return Object.freeze(copy);
}

const ELECTRA_IDL_SHA256 = '70cc917357c2a589063f1efca80b2da0390f3c12c58eecb0b9367228c4bce1b0';
const CHECKPOINT_IDL_SHA256 = '2e5d5576576bdbbf8bfe7c3a9f83f48be400e602bf47d906d5ab3a7ff1f28b4f';
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
    if (!originalCanonical.eq(profile.sourceBlockHash) || !currentCanonical.eq(pin) || !observedCanonical.eq(observedHash)) {
      throw new Error('Original or observed finalized inbound source history changed');
    }
    const runtimeCode = await read(() => api.rpc.state.getStorage<Bytes>(':code', pin));
    if (!runtimeCode || runtimeCode.isEmpty || blake2AsHex(runtimeCode.toU8a(true), 256) !== profile.sourceRuntimeCodeHash.toLowerCase()) {
      throw new Error('Inbound proof source runtime identity mismatch');
    }
    observedHash = pin;
    observedHeight = height;
  };
  await authenticateSource(profile.sourceBlockHash);
  statusCb('Requesting transaction receipt', { txHash });
  const receipt = await read(() => ethClient.getTransactionReceipt(txHash));
  const block = await read(() => ethClient.getBlockByHash(receipt.blockHash));
  if (receipt.status !== 'success' || receipt.transactionHash.toLowerCase() !== txHash.toLowerCase() ||
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
    if (await read(() => checkpoint.serviceState.network().atBlock(pin).call()) !== profile.checkpoint.network) {
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
  const status = receipt.status === 'success' ? Uint8Array.from([1]) : Uint8Array.from([]);
  const cumulativeGasUsed = bigIntToBytes(receipt.cumulativeGasUsed);
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
    // Alloy RLP receipt envelopes wrap typed EIP-2718 bytes, but not legacy RLP lists.
    receiptRlp: targetReceipt.type === 'legacy' ? receipt : rlpEncode(receipt),
    root: trie.root(),
  };
}
