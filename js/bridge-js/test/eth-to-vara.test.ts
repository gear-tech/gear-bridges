import { beforeAll, afterAll, beforeEach, afterEach, test, expect, describe, vi } from 'vitest';
import { createPublicClient, PublicClient, webSocket, bytesToHex, hexToBytes, keccak256, TransactionReceipt, encodeEventTopics, encodeAbiParameters, parseAbiParameters } from 'viem';
import { GearApi, HexString } from '@gear-js/api';
import dotenv from 'dotenv';
import * as fs from 'fs';

import { BeaconClient, createBeaconClient, createEthereumClient, EthereumClient } from '../src/ethereum/index.js';
import { encodeEthToVaraEvent, HistoricalProxyClient, ProofResult, getPrefix } from '../src/vara/index.js';
import { composeProof, generateMerkleProof, immutableInboundProfile, InboundProofProfile } from '../src/eth-to-vara/proof-composer.js';
import { relayEthToVara, validateFinalizedEthToVaraReply, decodeConsumerReply, assertNativePendingReply, validateFinalizedNativeReconciliationReply } from '../src/eth-to-vara/relayer.js';
import type { ConsumerReplyContract } from '../src/eth-to-vara/relayer.js';
import { validateInboundTokenEffect, BridgingRequestedAbi, nativeOperationId, validateNativePendingCohort, validateNativeCohortContinuity, validateNativeRedemption,
  VFT_MANAGER_IDL_SHA256, NATIVE_WRAPPER_IDL_SHA256, type NativeReceiptDeposit } from '../src/eth-to-vara/token-effect.js';
import { encode as rlpEncode } from '@ethereumjs/rlp';
import { HistoricalProxy } from '../src/vara/historical-proxy.js';
import * as beaconMethods from '../src/ethereum/beacon-client.js';
import * as proofMethods from '../src/eth-to-vara/proof-composer.js';
import * as varaCodec from '../src/vara/eth-to-vara.js';
import { blake2AsHex } from '@polkadot/util-crypto';
import { ssz } from '@lodestar/types';
import { Sails } from 'sails-js';
import { SailsIdlParser } from 'sails-js-parser';
import { applicationAdmission, validateRuntimeProfile, type RuntimeProfile } from '../example/app.js';
import { readApprovedFixtureProfile } from './setup/setup.js';
import { tmpdir } from 'node:os';
import * as path from 'node:path';
import { createHash } from 'node:crypto';

dotenv.config();

const hash = (value: number): HexString => ('0x' + value.toString(16).padStart(64, '0')) as HexString;
const codec = (value: HexString) => ({ toHex: () => value, toString: () => value, eq: (other: unknown) => value === (typeof other === 'string' ? other : (other as { toHex(): string }).toHex()) });

async function managerRegistry() {
  const idl = fs.readFileSync(new URL('../../../api/gear/vft_manager.idl', import.meta.url), 'utf8');
  expect(createHash('sha256').update(idl).digest('hex')).toBe(VFT_MANAGER_IDL_SHA256);
  return new Sails(await SailsIdlParser.new()).parseIdl(idl).registry;
}

function originalReplyFixture() {
  const proxy = hash(1), sender = hash(2), msgId = hash(3), txHash = hash(4);
  const registry = new HistoricalProxyClient({} as GearApi, proxy).registry;
  const receiptRlp = Uint8Array.of(2, 0xf8, 0, 1);
  const innerReply = getPrefix('Ping', 'SubmitReceipt') + '010a';
  const requestPayload = registry.createType('(String, String, u64, Vec<u8>, [u8;32], Vec<u8>)',
    ['HistoricalProxy', 'Redirect', 100, '0x0102', hash(5), getPrefix('Ping', 'SubmitReceipt')]).toHex();
  const replyPayload = registry.createType('(String, String, Result<(Vec<u8>, Vec<u8>), ProxyError>)',
    ['HistoricalProxy', 'Redirect', { ok: [bytesToHex(receiptRlp), innerReply] }]).toHex();
  const state = { finalized: 12, requestCanonical: true, source: proxy, destination: sender, runtimeSuccess: true,
    payload: replyPayload, replyTo: msgId, requestPayload, queuedSource: sender, queuedDestination: proxy };
  const phase = { isApplyExtrinsic: true, asApplyExtrinsic: { toNumber: () => 0 } };
  const header = (height: number) => ({ number: { toBigInt: () => BigInt(height) }, parentHash: codec(hash(height - 1)) });
  const events = (height: number) => height === 10 ? [
    { phase, event: { section: 'gear', method: 'MessageQueued', data: [codec(msgId), codec(state.queuedSource), codec(state.queuedDestination)] } },
    { phase, event: { section: 'system', method: 'ExtrinsicSuccess', data: [] } },
  ] : height === 12 ? [{ phase: { isApplyExtrinsic: false }, event: { section: 'gear', method: 'UserMessageSent', data: [{
    id: codec(hash(8)), source: codec(state.source), destination: codec(state.destination),
    details: { isNone: false, unwrap: () => ({ to: codec(state.replyTo), code: { isSuccess: state.runtimeSuccess } }) },
    payload: registry.createType('Bytes', state.payload),
  }] } }] : [];
  const api = {
    createType: (name: string, value: string) => registry.createType(name, value),
    blocks: {
      get: async () => ({ block: { header: header(10), extrinsics: [{ hash: codec(txHash), isSigned: true, signer: { ...codec(sender), toHex: () => '0x00' + sender.slice(2) },
        method: { section: 'gear', method: 'sendMessage', args: [codec(proxy), codec(state.requestPayload), {}, { toString: () => '0' }] } }] } }),
      getBlockHash: async (height: bigint) => codec(hash(Number(height) === 10 && !state.requestCanonical ? 99 : Number(height))),
      getEvents: async (pin: HexString) => events(Number(BigInt(pin))),
    },
    rpc: { chain: {
      getHeader: async (pin: HexString) => header(Number(BigInt(pin))),
      getFinalizedHead: async () => hash(state.finalized),
      subscribeFinalizedHeads: async () => () => {},
    } },
    events: {
      gear: {
        MessageQueued: { is: (event: { section: string; method: string }) => event.section === 'gear' && event.method === 'MessageQueued' },
        UserMessageSent: { is: (event: { section: string; method: string }) => event.section === 'gear' && event.method === 'UserMessageSent' },
      },
      system: {
        ExtrinsicSuccess: { is: (event: { section: string; method: string }) => event.section === 'system' && event.method === 'ExtrinsicSuccess' },
        ExtrinsicFailed: { is: () => false },
      },
    },
  } as unknown as GearApi;
  return { state, registry, innerReply, receiptRlp, params: { gearApi: api, historicalProxyId: proxy, sender, msgId,
    blockHash: hash(10), txHash, requestPayload, receiptRlp, deadline: Date.now() + 1000 } };
}

  test('SDK timed-out Gear signing never submits after the original deadline', async () => {
    vi.useFakeTimers();
    let finishSigning!: () => void;
    let signingStarted!: () => void;
    const started = new Promise<void>((resolve) => { signingStarted = resolve; });
    const signature = new Promise<void>((resolve) => { finishSigning = resolve; });
    let broadcasts = 0;
    const extrinsic = { args: [codec(hash(1)), codec('0x0102')], hash: codec(hash(4)),
      signAsync: async () => { signingStarted(); await signature; return extrinsic; },
      send: async () => { broadcasts++; return () => {}; } };
    const transaction = { extrinsic, gasInfo: { min_limit: { toBigInt: () => 100n } },
      withAccount: () => transaction, calculateGas: async () => transaction,
      withGas: () => transaction, withValue: () => transaction,
      signAndSend: async () => { await extrinsic.signAsync(); await extrinsic.send(); } };
    const receipt = { blockNumber: 5n, blockHash: hash(5), status: 'success' };
    try {
      vi.spyOn(beaconMethods, 'createBeaconClient').mockResolvedValue({ genesisBlockTime: 0 } as BeaconClient);
      vi.spyOn(proofMethods, 'composeProof').mockResolvedValue({ proofBlock: { block: { slot: 100n } } } as ProofResult);
      vi.spyOn(varaCodec, 'encodeEthToVaraEvent').mockReturnValue('0x0102');
      vi.spyOn(HistoricalProxy.prototype, 'redirect').mockReturnValue(transaction as unknown as ReturnType<HistoricalProxy['redirect']>);
      const relay = relayEthToVara({ deadline: Date.now() + 1000,
        gearApi: { blockGasLimit: { toBigInt: () => 10000n } } as GearApi, signer: hash(2),
        transactionHash: hash(4), beaconRpcUrl: 'https://beacon.invalid', historicalProxyId: hash(1),
        clientId: hash(3), clientServiceName: 'Ping', clientMethodName: 'SubmitReceipt',
        inboundProfile: { ...checkpointProofFixture().profile, ethereumGenesisHash: hash(8), historicalProxyId: hash(1),
          consumer: { ...checkpointProofFixture().profile.consumer, programId: hash(3) } },
        consumerReply: { registry: new HistoricalProxyClient({} as GearApi, hash(1)).registry,
          resultType: 'Result<Null,String>', idlSha256: checkpointProofFixture().profile.consumer.idlSha256,
          verifyEffect: async () => { throw new Error('Signing test must not reach settlement'); } },
        ethereumPublicClient: { getChainId: async () => 1, getTransactionReceipt: async () => receipt,
          getBlock: async ({ blockNumber, blockTag }: { blockNumber?: bigint; blockTag?: string }) =>
            blockTag ? { number: 6n } : { hash: blockNumber === 0n ? hash(8) : hash(5) } } as unknown as PublicClient,
      }).catch((error: Error) => error);
      await started;
      await vi.advanceTimersByTimeAsync(1001);
      expect(await relay).toBeInstanceOf(Error);
      expect((await relay as Error).message).toContain('deadline expired');
      finishSigning();
      await Promise.resolve();
      await Promise.resolve();
      expect(broadcasts).toBe(0);
    } finally { vi.restoreAllMocks(); vi.useRealTimers(); }
  });

test('SDK final effect pin readback remains bounded by the original deadline', async () => {
  vi.useFakeTimers();
  const fixture = originalReplyFixture(), api = fixture.params.gearApi;
  const profile = { ...checkpointProofFixture().profile, ethereumGenesisHash: hash(8),
    consumer: { ...checkpointProofFixture().profile.consumer, programId: hash(5) } };
  const inner = getPrefix('Ping', 'SubmitReceipt') + '00';
  fixture.state.payload = fixture.registry.createType('(String,String,Result<(Vec<u8>,Vec<u8>),ProxyError>)',
    ['HistoricalProxy', 'Redirect', { ok: [bytesToHex(fixture.receiptRlp), inner] }]).toHex();
  let effectVerified = false, readStarted!: () => void;
  const started = new Promise<void>(resolve => { readStarted = resolve; });
  const canonical = api.blocks.getBlockHash;
  api.blocks.getBlockHash = async (height) => {
    if (effectVerified) { readStarted(); return new Promise<never>(() => {}); }
    return canonical(height);
  };
  Object.assign(api, { blockGasLimit: { toBigInt: () => 10000n },
    programStorage: { getProgram: async () => ({ state: { isInitialized: true }, codeId: codec(profile.consumer.codeId) }) } });
  Object.assign(api.rpc, { state: { getStorage: async () => ({ isEmpty: false, toU8a: () => Uint8Array.of(1, 2, 3) }) } });
  const extrinsic = { args: [codec(hash(1)), codec(fixture.state.requestPayload)], hash: codec(hash(4)),
    signAsync: async () => extrinsic, send: async (callback: (result: unknown) => void) => {
      callback({ events: await api.blocks.getEvents(hash(10)), status: { isInBlock: true, asInBlock: codec(hash(10)) } });
      return () => {};
    } };
  const transaction = { extrinsic, gasInfo: { min_limit: { toBigInt: () => 100n } },
    withAccount: () => transaction, calculateGas: async () => transaction, withGas: () => transaction, withValue: () => transaction };
  try {
    vi.spyOn(beaconMethods, 'createBeaconClient').mockResolvedValue({ genesisBlockTime: 0 } as BeaconClient);
    vi.spyOn(proofMethods, 'composeProof').mockResolvedValue({ proofBlock: { block: { slot: 100 } }, receiptRlp: fixture.receiptRlp } as ProofResult);
    vi.spyOn(varaCodec, 'encodeEthToVaraEvent').mockReturnValue('0x0102');
    vi.spyOn(HistoricalProxy.prototype, 'redirect').mockReturnValue(transaction as unknown as ReturnType<HistoricalProxy['redirect']>);
    const relay = relayEthToVara({ deadline: Date.now() + 1000, gearApi: api, signer: hash(2),
      transactionHash: hash(4), beaconRpcUrl: 'https://beacon.invalid', historicalProxyId: hash(1),
      clientId: hash(5), clientServiceName: 'Ping', clientMethodName: 'SubmitReceipt', inboundProfile: profile,
      consumerReply: { registry: fixture.registry, resultType: 'Result<Null,String>', idlSha256: profile.consumer.idlSha256,
        verifyEffect: async () => { effectVerified = true; } },
      ethereumPublicClient: { getChainId: async () => 1, getTransactionReceipt: async () => ({ blockNumber: 5n, blockHash: hash(5) }),
        getBlock: async ({ blockNumber, blockTag }: { blockNumber?: bigint; blockTag?: string }) =>
          blockTag ? { number: 6n } : { hash: blockNumber === 0n ? hash(8) : hash(5) } } as unknown as PublicClient,
    }).catch((error: Error) => error);
    await started;
    await vi.advanceTimersByTimeAsync(1001);
    expect((await relay as Error).message).toContain('deadline expired');
  } finally { vi.restoreAllMocks(); vi.useRealTimers(); }
});


describe('SDK inbound acceptance', () => {
  test('returns original finalized raw inner Err bytes without advertising application success, including on resume', async () => {
    const fixture = originalReplyFixture();
    const result = await validateFinalizedEthToVaraReply(fixture.params);
    expect(result.clientReply).toBe(fixture.innerReply);
    expect(result).not.toHaveProperty('ok');
    expect(result.replyBlockHash).toBe(hash(12));
    expect(result.replyMessageId).toBe(hash(8));
    expect(await result.isFinalized).toBe(true);
    const resumed = await validateFinalizedEthToVaraReply(fixture.params);
    expect(resumed.clientReply).toBe(result.clientReply);
    expect(resumed.replyBlockHash).toBe(result.replyBlockHash);
    expect(resumed.msgId).toBe(result.msgId);
  });

  test.each(['source', 'destination', 'queuedSource', 'queuedDestination', 'runtime', 'route', 'trailing', 'receipt', 'request', 'canonical'])(
    'rejects mismatched original %s evidence', async (kind) => {
      const fixture = originalReplyFixture();
      if (kind === 'source') fixture.state.source = hash(99);
      if (kind === 'destination') fixture.state.destination = hash(99);
      if (kind === 'queuedSource') fixture.state.queuedSource = hash(99);
      if (kind === 'queuedDestination') fixture.state.queuedDestination = hash(99);
      if (kind === 'runtime') fixture.state.runtimeSuccess = false;
      if (kind === 'route') fixture.state.payload = fixture.registry.createType('(String, String, Result<(Vec<u8>, Vec<u8>), ProxyError>)',
        ['HistoricalProxy', 'Relayed', { ok: [bytesToHex(fixture.receiptRlp), fixture.innerReply] }]).toHex();
      if (kind === 'trailing') fixture.state.payload = (fixture.state.payload + '00') as HexString;
      if (kind === 'receipt') fixture.params.receiptRlp = Uint8Array.of(1);
      if (kind === 'request') fixture.state.requestPayload = (fixture.state.requestPayload + '00') as HexString;
      if (kind === 'canonical') fixture.state.requestCanonical = false;
      await expect(validateFinalizedEthToVaraReply(fixture.params)).rejects.toThrow();
    });

  test('request finality cannot accept an unfinalized original reply or reopen an expired deadline', async () => {
    const fixture = originalReplyFixture();
    fixture.state.finalized = 10;
    fixture.params.deadline = Date.now() + 30;
    await expect(validateFinalizedEthToVaraReply(fixture.params)).rejects.toThrow();
    fixture.state.finalized = 12;
    await expect(validateFinalizedEthToVaraReply(fixture.params)).rejects.toThrow('deadline expired');
  });

  test('a finalized outer error remains a proxy error rather than client bytes', async () => {
    const fixture = originalReplyFixture();
    fixture.state.payload = fixture.registry.createType('(String, String, Result<(Vec<u8>, Vec<u8>), ProxyError>)',
      ['HistoricalProxy', 'Redirect', { err: { SendFailure: 'no effect' } }]).toHex();
    const result = await validateFinalizedEthToVaraReply(fixture.params);
    expect(result.error).toEqual({ SendFailure: 'no effect' });
    expect(result.clientReply).toBeUndefined();
  });

  test('preserves the actual nested proof-error variant rather than lowercased JSON aliases', async () => {
    const fixture = originalReplyFixture();
    fixture.state.payload = fixture.registry.createType('(String, String, Result<(Vec<u8>, Vec<u8>), ProxyError>)',
      ['HistoricalProxy', 'Redirect', { err: { EthereumEventClient: 'InvalidReceiptProof' } }]).toHex();
    const result = await validateFinalizedEthToVaraReply(fixture.params);
    expect(result.error).toEqual({ EthereumEventClient: 'InvalidReceiptProof' });
    expect(result.clientReply).toBeUndefined();
  });

  test('a stalled original-block RPC remains bounded by the same original deadline', async () => {
    const fixture = originalReplyFixture();
    fixture.params.gearApi.blocks.get = async () => new Promise<never>(() => {});
    fixture.params.deadline = Date.now() + 30;
    await expect(validateFinalizedEthToVaraReply(fixture.params)).rejects.toThrow('deadline expired');
  });

  test('bounded ordered header retrieval omits only absent slots', async () => {
    let active = 0, peak = 0;
    const fetch = vi.spyOn(globalThis, 'fetch').mockImplementation(async (url) => {
      if (String(url).endsWith('/genesis')) return new Response(JSON.stringify({ data: { genesis_time: '0', genesis_validators_root: hash(1), genesis_fork_version: '0x00000000' } }));
      const slot = Number(String(url).split('/').pop());
      active++; peak = Math.max(peak, active);
      await new Promise((resolve) => setTimeout(resolve, slot % 3));
      active--;
      return slot === 11 ? new Response('', { status: 404 }) : new Response(JSON.stringify({ data: { header: { message: { slot: String(slot) } } } }));
    });
    try {
      const client = await createBeaconClient('https://owned-fixture.invalid');
      const headers = await client.requestHeaders(5, 42);
      expect(headers.map((item) => Number(item.header.message.slot))).toEqual(Array.from({ length: 38 }, (_, i) => i + 5).filter((slot) => slot !== 11));
      expect(peak).toBeLessThanOrEqual(16);
    } finally { fetch.mockRestore(); }
  });

  test.each([429, 503])('preserves non-absence Beacon failure %s instead of silently skipping history', async (status) => {
    const fetch = vi.spyOn(globalThis, 'fetch').mockImplementation(async (url) => String(url).endsWith('/genesis')
      ? new Response(JSON.stringify({ data: { genesis_time: '0', genesis_validators_root: hash(1), genesis_fork_version: '0x00000000' } }))
      : new Response('', { status }));
    try {
      const client = await createBeaconClient('https://owned-fixture.invalid');
      await expect(client.requestHeaders(5, 6)).rejects.toThrow();
    } finally { fetch.mockRestore(); }
  });

  test.each(['legacy', 'eip2930', 'eip1559', 'eip4844', 'eip7702'] as const)(
    'preserves %s Alloy envelope framing while authenticating the raw EIP-2718 trie value', async (type) => {
      const receipt = { type, transactionIndex: 0, status: 'success', cumulativeGasUsed: 21000n,
        logsBloom: '0x' + '00'.repeat(256), logs: [] } as unknown as TransactionReceipt;
      const legacy = 'f9010801825208b90100' + '00'.repeat(256) + 'c0';
      const tag = ['legacy', 'eip2930', 'eip1559', 'eip4844', 'eip7702'].indexOf(type);
      const trieValue = type === 'legacy' ? legacy : '0' + tag + legacy;
      const expectedEnvelope = type === 'legacy' ? legacy : 'b9010c' + trieValue;
      const result = await generateMerkleProof(0, [receipt]);
      expect(bytesToHex(result.receiptRlp)).toBe('0x' + expectedEnvelope);
      const leaf = rlpEncode([Uint8Array.of(0x20, 0x80), hexToBytes(('0x' + trieValue) as HexString)]);
      expect(bytesToHex(result.root)).toBe(keccak256(leaf));
      expect(result.proof.map((node) => bytesToHex(node))).toEqual([bytesToHex(leaf)]);
    });
});

describe('SDK finalized consumer and token effects', () => {
  test('snapshots a complete immutable profile and rejects missing deployment pins', () => {
    const original = checkpointProofFixture().profile;
    const snapshot = immutableInboundProfile(original);
    Object.defineProperty(original.consumer, 'codeId', { value: hash(99) });
    expect(snapshot.consumer.codeId).not.toBe(hash(99));
    expect(Object.isFrozen(snapshot.consumer)).toBe(true);
    expect(() => immutableInboundProfile({ ...snapshot, historicalProxyIdlSha256: '' })).toThrow('HOLD');
    expect(() => immutableInboundProfile({ ...snapshot, endpoint: { ...snapshot.endpoint, idlSha256: 'b'.repeat(64) } })).toThrow('HOLD');
    expect(() => immutableInboundProfile(undefined as unknown as InboundProofProfile)).toThrow('HOLD');
  });

  test('only the complete expected consumer tuple and inner Ok report success', () => {
    const profile = checkpointProofFixture().profile;
    const registry = new HistoricalProxyClient({} as GearApi, hash(1)).registry;
    registry.register({ ConsumerError: { _enum: ['Rejected'] } });
    const contract = { registry, resultType: 'Result<Null, ConsumerError>', idlSha256: profile.consumer.idlSha256,
      verifyEffect: async () => { throw new Error('Codec-only test must not dispatch an effect'); } };
    const encode = (service: string, method: string, value: unknown) => registry.createType(
      '(String, String, Result<Null, ConsumerError>)', [service, method, value]).toHex();
    const ok = encode('Ping', 'SubmitReceipt', { Ok: null });
    expect(decodeConsumerReply(ok, profile, contract)).toBeNull();
    for (const payload of [ok + '00', encode('Other', 'SubmitReceipt', { Ok: null }),
      encode('Ping', 'Other', { Ok: null }), encode('Ping', 'SubmitReceipt', { Err: 'Rejected' })]) {
      expect(() => decodeConsumerReply(payload as HexString, profile, contract)).toThrow();
    }
    expect(() => decodeConsumerReply(ok, profile, { ...contract, idlSha256: 'b'.repeat(64) })).toThrow();
  });

  for (const fast of [false, true]) test('admits a separately labeled ' + (fast ? 'fast' : 'normal') + ' run only with independent runtime approval and actual actor profile pins', () => {
    const parent = fs.mkdtempSync(path.join(fs.realpathSync(tmpdir()), 'sdk-normal-admission-'));
    const runId = '6a116fc1-a332-4e2a-8f1f-56917de177bb', root = path.join(parent, runId), campaignName = 'hoodi-' + (fast ? 'fast' : 'normal') + '-runtime-unit';
    const artifacts = path.join(root, 'app-messages/artifacts'), sealed = path.join(root, 'sealed');
    fs.mkdirSync(artifacts, { recursive: true }); fs.mkdirSync(sealed, { recursive: true }); fs.mkdirSync(path.join(root, 'supervisors'));
    const digest = (bytes: string | Uint8Array) => createHash('sha256').update(bytes).digest('hex');
    const store = (filename: string, value: unknown) => { fs.mkdirSync(path.dirname(filename), { recursive: true }); const bytes = JSON.stringify(value, (_, value) => typeof value === 'bigint' ? value.toString() : value); fs.writeFileSync(filename, bytes, { mode: 0o600 }); return digest(bytes); };
    try {
      const forks = [{ name: 'phase0', epoch: 0n, version: '0x10000910' }, { name: 'altair', epoch: 0n, version: '0x20000910' },
        { name: 'bellatrix', epoch: 0n, version: '0x30000910' }, { name: 'capella', epoch: 0n, version: '0x40000910' },
        { name: 'deneb', epoch: 0n, version: '0x50000910' }, { name: 'electra', epoch: 2048n, version: '0x60000910' },
        { name: 'fulu', epoch: 50688n, version: '0x70000910' }] as InboundProofProfile['forks'];
      const approved = immutableInboundProfile({ ...nativeProfile(), ethereumChainId: 560048n, ethereumGenesisHash: '0xbbe312868b376a3001692a646dd2d7d1e4406380dfd86b98aa8a34d1557c971b',
        beaconGenesisValidatorsRoot: '0x212f13fc4df078b6cb7db228f1c8307566dcecf900867401a92023d7ba99cb5f', beaconGenesisTime: 1742213400n,
        sourceGenesisHash: hash(999), forks });
      const runtime: RuntimeProfile = { name: fast ? 'fast-runtime-hoodi' : 'normal-runtime-hoodi', runtimeCommit: '19ab81dd208b3ce7b339f5d1dbd144511d3c7e2c', runtimePullRequest: 5642,
        slotDurationMs: 3000, epochDurationBlocks: fast ? 64 : 2400, warmupDurationMs: fast ? 3600000 : 18000000, requiredAuthorityHandovers: 2, tokenBatchDurationMs: 3600000,
        ...(fast ? { functionalOnly: true, cadencePatchSha256: digest('cadence-only patch') } : {}),
        applicationAttemptDurationMs: 2640000, testOnly: true, executionAuthorized: false, releaseQualified: false, runtimeCiStatus: 'unresolved',
        gearBinarySha256: digest('unit binary'), runtimeCodeSha256: digest(Uint8Array.of(1,2,3)), runtimeCodeBlake2b256: approved.sourceRuntimeCodeHash.slice(2),
        runtimeCodeKeccak256: keccak256(Uint8Array.of(1,2,3)).slice(2), approvalSha256: '' };
      const { approvalSha256: _unused, ...approvalProfile } = runtime;
      runtime.approvalSha256 = store(path.join(sealed, 'runtime-approval.json'), { status: 'APPROVED_FOR_TEST_IMPLEMENTATION', runtimeProfile: approvalProfile });
      const run = { schemaVersion: 1, testOnly: true, runId, bundle: { path: sealed, sha256: digest('selected sealed manifest') }, runtimeProfile: runtime,
        network: { chainId: 560048, genesisHash: approved.ethereumGenesisHash, executionHttp: 'https://unit.invalid', executionWss: 'wss://unit.invalid', beaconHttp: 'https://unit.invalid' },
        source: { aliceRpc: 'ws://127.0.0.1:10001', bobRpc: 'ws://127.0.0.1:10002' } };
      const core = { anchor: { sourceGenesis: approved.sourceGenesisHash, bridgeDomain: hash(33), beefyActivationBlock: 40, mmrStartBlock: 45, domainBindingBlock: 48 },
        ethereum: { queue: ('0x' + '01'.repeat(20)) as HexString, client: ('0x' + '02'.repeat(20)) as HexString, verifier: ('0x' + '03'.repeat(20)) as HexString, bytecodeHashes: {} }, ethereumConfiguration: { tokens: [] }, gearManager: approved.consumer.programId };
      const stack = { checkpoint: approved.checkpoint.programId, sourceGenesis: approved.sourceGenesisHash, programs: { historicalProxy: { id: approved.historicalProxyId }, ethEventsElectra: { id: approved.endpoint.programId } } };
      const identity = { runtimeProfile: runtime, runtimeCodeSha256: runtime.runtimeCodeSha256, runtimeCodeBlake2b256: runtime.runtimeCodeBlake2b256,
        runtimeCodeKeccak256: runtime.runtimeCodeKeccak256, ...core.anchor };
      const preservedFiles = { 'source-chain/launch-state.json': store(path.join(root, 'source-chain/launch-state.json'), { phase: 'ready', identity,
        readiness: { genesisHash: approved.sourceGenesisHash, commonFinalized: { hash: approved.sourceBlockHash } }, pinned: { sourceIdentity: { ...core.anchor, genesisHash: approved.sourceGenesisHash } } }),
        'deployment.json': store(path.join(root, 'deployment.json'), core), 'token-stack/token-stack.json': store(path.join(root, 'token-stack/token-stack.json'), stack) };
      store(path.join(root, 'supervisors/normal-campaign-admission.json'), { schemaVersion: 1, testOnly: true, runId, campaignName, bundleSha256: run.bundle.sha256, runtimeProfile: runtime, automaticRerun: false, launched: false, preservedFiles });
      const build = { schemaVersion: 1, testOnly: true, runId, campaignName, files: { 'inbound-proof-profile.json': store(path.join(artifacts, 'inbound-proof-profile.json'), approved),
        'vft_manager.idl': approved.consumer.idlSha256, 'checkpoint_light_client.idl': approved.checkpoint.idlSha256, 'eth_events_electra.idl': approved.endpoint.idlSha256, 'historical_proxy.idl': approved.historicalProxyIdlSha256 } };
      const bundle = { runtimeProfile: runtime, files: { 'bin/gear': runtime.gearBinarySha256, 'runtime-approval.json': runtime.approvalSha256 } }, qualification = { runtimeProfile: runtime };
      expect(applicationAdmission(root, run, bundle, qualification, artifacts, build, core, stack, campaignName).sourceGenesisHash).toBe(hash(999));
      expect(readApprovedFixtureProfile(path.join(artifacts, 'inbound-proof-profile.json'), approved.historicalProxyId).sourceGenesisHash).toBe(hash(999));
      expect(() => readApprovedFixtureProfile(path.join(artifacts, 'inbound-proof-profile.json'), hash(555))).toThrow();
      for (const change of [{ runtimeCommit: 'f961bed815dd4ab0802703620605ea3b3659ac60' }, { slotDurationMs: 400 }, { runtimePullRequest: 5644 },
        { warmupDurationMs: fast ? 18000000 : 3600000 }, { epochDurationBlocks: fast ? 2400 : 64 }, { requiredAuthorityHandovers: 1 },
        { functionalOnly: !fast }, { cadencePatchSha256: '0'.repeat(64) }, { cadencePatchSha256: undefined },
        { executionAuthorized: true }, { releaseQualified: true }, { approvalSha256: '0'.repeat(64) }]) {
        expect(() => validateRuntimeProfile({ ...runtime, ...change }, runtime.gearBinarySha256)).toThrow();
      }
      expect(() => applicationAdmission(root, run, bundle, qualification, artifacts, build, core, stack,
        'hoodi-' + (fast ? 'normal' : 'fast') + '-runtime-unit')).toThrow();
      expect(() => applicationAdmission(root, run, bundle, { runtimeProfile: { ...runtime, runtimeCiStatus: 'passed' } }, artifacts, build, core, stack, campaignName)).toThrow();
      fs.unlinkSync(path.join(artifacts, 'inbound-proof-profile.json'));
      expect(() => applicationAdmission(root, run, bundle, qualification, artifacts, build, core, stack, campaignName)).toThrow();
    } finally { fs.rmSync(parent, { recursive: true, force: true }); }
  });


  test('recognizes only the actual typed NativeSettlementPending without relabeling it success', async () => {
    const profile = nativeProfile(), registry = await managerRegistry();
    const contract: ConsumerReplyContract = { registry, resultType: 'Result<Null, Error>', idlSha256: VFT_MANAGER_IDL_SHA256,
      verifyEffect: async () => { throw new Error('Classifier must not verify or dispatch an effect'); },
      nativeSettlement: { expectedEffect: { managerAddress: `0x${'12'.repeat(20)}`, sourceToken: `0x${'13'.repeat(20)}`, destinationToken: hash(8), sender: `0x${'14'.repeat(20)}`, receiver: hash(9), amount: 10n },
        readDeposits: async () => [], readRedemption: async () => null, readReceiptStatus: async () => 'Reserved' as const } };
    const encode = (error: string) => registry.createType('(String,String,Result<Null,Error>)', ['VftManager','SubmitReceipt',{ err: { [error]: null } }]).toHex();
    const pending = encode('NativeSettlementPending');
    expect(() => assertNativePendingReply(pending, profile, contract)).not.toThrow();
    expect(() => decodeConsumerReply(pending, profile, contract)).toThrow();
    for (const error of ['NativeSettlementReturned', 'MessageFailed', 'ReceiptLeaseActive']) expect(() => assertNativePendingReply(encode(error), profile, contract)).toThrow();
    expect(() => assertNativePendingReply(pending, profile, { ...contract, nativeSettlement: undefined })).toThrow();
    expect(() => assertNativePendingReply(pending, { ...profile, nativeWrapper: undefined }, contract)).toThrow();
    expect(() => assertNativePendingReply((pending + '00') as HexString, profile, contract)).toThrow();
  });

  test('reconciles exact native cohorts only after original payout delivery, preserving operation and both child identities', async () => {
    const expected = { managerAddress: ('0x' + '12'.repeat(20)) as HexString, sourceToken: ('0x' + '13'.repeat(20)) as HexString,
      destinationToken: hash(8), sender: ('0x' + '14'.repeat(20)) as HexString, receiver: hash(9), amount: 10n };
    const logs = [10n, 20n].map(amount => ({ address: expected.managerAddress, topics: encodeEventTopics({ abi: BridgingRequestedAbi, eventName: 'BridgingRequested',
      args: { from: expected.sender, to: expected.receiver, token: expected.sourceToken } }), data: encodeAbiParameters(parseAbiParameters('uint256'), [amount]) }));
    const { receiptRlp } = await generateMerkleProof(0, [{ type: 'eip1559', transactionIndex: 0, status: 'success', cumulativeGasUsed: 21000n,
      logsBloom: '0x' + '00'.repeat(256), logs } as unknown as TransactionReceipt]);
    const identity = { managerId: hash(1), proxyId: hash(2), wrapperId: hash(8), slot: 100n, transactionIndex: 3n, receiptRlp, expectedEffect: expected };
    const rows: NativeReceiptDeposit[] = [10n, 20n].map((amount, log_index) => ({ log_index, sender: expected.sender, receiver: expected.receiver,
      eth_token_id: expected.sourceToken, token_id: expected.destinationToken, amount, outcome: 'NativeQueued', native: true, supply: 'Gear',
      operation_id: nativeOperationId(identity, BigInt(log_index)), child: hash(40 + log_index) }));
    expect(() => validateNativePendingCohort(identity, rows)).not.toThrow();
    const registry = await managerRegistry();
    const tuple = registry.createType('([u8;21],[u8;32],[u8;32],H160,u64,u64,u64,H256)', [bytesToHex(new TextEncoder().encode('vara/native-escrow/v1')),
      identity.managerId, identity.proxyId, expected.managerAddress, identity.slot, identity.transactionIndex, 0, keccak256(receiptRlp)]).toU8a();
    expect(rows[0].operation_id).toBe(keccak256(tuple));
    expect(rows[1].operation_id).not.toBe(rows[0].operation_id);
    for (const outcome of ['Unknown', 'Pending', 'InFlight', 'Rejected']) expect(() => validateNativePendingCohort(identity, [{ ...rows[0], outcome }, rows[1]])).toThrow();
    for (const change of [{ child: null }, { native: false }, { supply: 'Ethereum' as const }, { operation_id: hash(99) }, { amount: 11n }]) {
      expect(() => validateNativePendingCohort(identity, [{ ...rows[0], ...change }, rows[1]])).toThrow();
    }
    const delivered = rows.map(row => ({ ...row, outcome: 'Settled' }));
    expect(() => validateNativeCohortContinuity(rows, delivered, true)).not.toThrow();
    expect(() => validateNativeCohortContinuity(rows, rows, true)).toThrow();
    expect(() => validateNativePendingCohort(identity, delivered)).not.toThrow();
    for (const change of [{ child: hash(99) }, { operation_id: hash(99) }, { amount: 11n }, { receiver: hash(99) }, { supply: 'Ethereum' as const }]) {
      expect(() => validateNativeCohortContinuity(rows, [{ ...delivered[0], ...change }, delivered[1]], true)).toThrow();
    }
    expect(() => validateNativeCohortContinuity(rows, delivered.slice(0, 1), true)).toThrow();
    const queued = { from: identity.managerId, to: expected.receiver, amount: 10n, child: hash(50), status: 'Queued' as const, returned_value: 0n };
    expect(validateNativeRedemption(rows[0], identity.managerId, queued)).toBe(false);
    const settled = { ...queued, status: 'Delivered' as const };
    expect(validateNativeRedemption(delivered[0], identity.managerId, settled, queued)).toBe(true);
    for (const change of [{ child: hash(99) }, { child: rows[0].child! }, { returned_value: 1n }, { status: 'Returned' as const }, { status: 'Ambiguous' as const },
      { to: hash(99) }, { from: hash(99) }, { amount: 11n }]) expect(() => validateNativeRedemption(delivered[0], identity.managerId, { ...settled, ...change }, queued)).toThrow();
    expect(() => validateNativeRedemption(rows[0], identity.managerId, null)).toThrow();
    expect(() => validateInboundTokenEffect(receiptRlp, rows, expected)).toThrow();
    expect(() => validateInboundTokenEffect(receiptRlp, delivered, expected)).not.toThrow();
  });

  test('authenticates the separately signed original ReconcileReceipt reply and exact receipt coordinates', async () => {
    const fixture = originalReplyFixture(), profile = nativeProfile(), registry = await managerRegistry();
    const requestPayload = registry.createType('(String,String,u64,u64)', ['VftManager', 'ReconcileReceipt', 100, 3]).toHex();
    fixture.state.requestPayload = requestPayload;
    fixture.state.payload = registry.createType('(String,String,Result<ReceiptStatus,Error>)', ['VftManager','ReconcileReceipt',{ ok: 'Processed' }]).toHex();
    const params = { ...fixture.params, registry, inboundProfile: profile, slot: 100n, transactionIndex: 3n, requestPayload };
    const result = await validateFinalizedNativeReconciliationReply(params);
    expect(result.txHash).toBe(fixture.params.txHash); expect(result.msgId).toBe(fixture.params.msgId); expect(result.replyBlockHash).toBe(hash(12));
    expect(() => validateFinalizedNativeReconciliationReply({ ...params, transactionIndex: 4n })).toThrow();
    fixture.state.payload = registry.createType('(String,String,Result<ReceiptStatus,Error>)', ['VftManager','ReconcileReceipt',{ ok: 'Reserved' }]).toHex();
    await expect(validateFinalizedNativeReconciliationReply(params)).rejects.toThrow();
    fixture.state.payload = registry.createType('(String,String,Result<ReceiptStatus,Error>)', ['VftManager','ReconcileReceipt',{ ok: 'Processed' }]).toHex();
    fixture.state.requestCanonical = false;
    await expect(validateFinalizedNativeReconciliationReply(params)).rejects.toThrow();
  });


  test.each(['legacy', 'eip2930', 'eip1559', 'eip4844', 'eip7702'] as const)(
    'authenticates every original %s manager log and exact settled multi-log effects', async (type) => {
      const address = (n: number) => ('0x' + n.toString(16).padStart(40, '0')) as HexString;
      const expected = { managerAddress: address(1), sourceToken: address(2), destinationToken: hash(3),
        sender: address(4), receiver: hash(5), amount: 10n };
      const logs = [10n, 20n].map(amount => ({ address: expected.managerAddress,
        topics: encodeEventTopics({ abi: BridgingRequestedAbi, eventName: 'BridgingRequested',
          args: { from: expected.sender, to: expected.receiver, token: expected.sourceToken } }),
        data: encodeAbiParameters(parseAbiParameters('uint256'), [amount]) }));
      const receipt = { type, transactionIndex: 0, status: 'success', cumulativeGasUsed: 21000n,
        logsBloom: '0x' + '00'.repeat(256), logs } as unknown as TransactionReceipt;
      const { receiptRlp } = await generateMerkleProof(0, [receipt]);
      const rows = [10n, 20n].map((amount, log_index) => ({ log_index, sender: expected.sender,
        receiver: expected.receiver, eth_token_id: expected.sourceToken, token_id: expected.destinationToken, amount, outcome: 'Settled' }));
      const identical = await generateMerkleProof(0, [{ ...receipt, logs: [receipt.logs[0], receipt.logs[0]] }]);
      expect(() => validateInboundTokenEffect(identical.receiptRlp, [rows[0], { ...rows[1], amount: 10n }], expected)).not.toThrow();
      expect(() => validateInboundTokenEffect(receiptRlp, rows, expected)).not.toThrow();
      for (const outcome of ['Pending', 'InFlight', 'Rejected', 'Unknown', 'NativeQueued']) {
        expect(() => validateInboundTokenEffect(receiptRlp, [{ ...rows[0], outcome }, rows[1]], expected)).toThrow();
      }
      for (const badRows of [rows.slice(0, 1), [rows[0], rows[0]], [{ ...rows[0], amount: 11n }, rows[1]],
        [{ ...rows[0], token_id: hash(99) }, rows[1]], [{ ...rows[0], receiver: hash(99) }, rows[1]]]) {
        expect(() => validateInboundTokenEffect(receiptRlp, badRows, expected)).toThrow();
      }
      expect(() => validateInboundTokenEffect(receiptRlp, rows, { ...expected, amount: 11n })).toThrow();
      const failed = await generateMerkleProof(0, [{ ...receipt, status: 'reverted' }]);
      expect(() => validateInboundTokenEffect(failed.receiptRlp, rows, expected)).toThrow();
      expect(() => validateInboundTokenEffect(new Uint8Array([...receiptRlp, 0]), rows, expected)).toThrow();
    });
});


function nativeProfile(): InboundProofProfile {
  const original = checkpointProofFixture().profile;
  return immutableInboundProfile({ ...original, consumer: { ...original.consumer, idlSha256: VFT_MANAGER_IDL_SHA256, service: 'VftManager' },
    nativeWrapper: { programId: hash(8), codeId: hash(18), idlSha256: NATIVE_WRAPPER_IDL_SHA256 } });
}


function checkpointProofFixture(fault = '') {
  const proxyId = hash(1), endpointId = hash(2), checkpointId = hash(3);
  const registry = new HistoricalProxyClient({} as GearApi, proxyId).registry;
  registry.register({ CheckpointError: { _enum: ['OutDated', 'NotPresent'] }, Network: { _enum: ['Mainnet', 'Sepolia', 'Holesky', 'Hoodi'] } });
  const runtime = Uint8Array.of(1, 2, 3);
  const receipt = { type: 'eip1559', transactionIndex: 0, transactionHash: hash(5), blockNumber: 5n, blockHash: hash(6),
    status: 'success', cumulativeGasUsed: 21000n, logsBloom: '0x' + '00'.repeat(256), logs: [] } as unknown as TransactionReceipt;
  const encodedReceipt = hexToBytes(('0x02f9010801825208b90100' + '00'.repeat(256) + 'c0') as HexString);
  const receiptsRoot = keccak256(rlpEncode([Uint8Array.of(0x20, 0x80), encodedReceipt]));
  const block = ssz.fulu.BeaconBlock.defaultValue();
  block.slot = 100;
  block.body.executionPayload.blockNumber = 5;
  block.body.executionPayload.blockHash = hexToBytes(receipt.blockHash);
  block.body.executionPayload.receiptsRoot = hexToBytes(receiptsRoot);
  const root = bytesToHex(ssz.fulu.BeaconBlock.hashTreeRoot(block));
  const profile: InboundProofProfile = { ethereumChainId: 1n, ethereumGenesisHash: hash(7), beaconGenesisValidatorsRoot: hash(8), beaconGenesisTime: 0n,
    sourceGenesisHash: hash(9), sourceBlockHash: hash(20), sourceRuntimeCodeHash: blake2AsHex(runtime, 256), historicalProxyId: proxyId, historicalProxyCodeId: hash(11),
    historicalProxyIdlSha256: '4bde230e4759abfedd44070790b3a579d7f75aadff14266e0f1a287f61942bc7',
    endpoint: { programId: endpointId, codeId: hash(12), idlSha256: '70cc917357c2a589063f1efca80b2da0390f3c12c58eecb0b9367228c4bce1b0', framing: 'electra' },
    checkpoint: { programId: checkpointId, codeId: hash(13), idlSha256: '2e5d5576576bdbbf8bfe7c3a9f83f48be400e602bf47d906d5ab3a7ff1f28b4f', network: 'Hoodi' },
    consumer: { programId: proxyId, codeId: hash(11), idlSha256: 'a'.repeat(64), service: 'Ping', method: 'SubmitReceipt' },
    forks: [{ name: 'electra', epoch: 0n, version: '0x01000000' }, { name: 'fulu', epoch: 1n, version: '0x02000000' }] };
  let release: (() => void) | undefined;
  const state = { absent: false, proofReads: 0, checkpointPins: [] as HexString[], release: () => release?.() };
  const api = {
    query: { gearProgram: { programStorage: async (_id: HexString, subscriber?: unknown) => subscriber ? () => {} : { isNone: false, unwrap: () => ({ isExited: false, isTerminated: false }) } } },
    genesisHash: codec(profile.sourceGenesisHash), blockGasLimit: { toBigInt: () => 10000n }, specVersion: 1,
    blocks: {
      getFinalizedHead: async () => codec(hash(state.absent && fault === 'regression' ? 19 : fault === 'intermediate-history' && state.checkpointPins.includes(hash(21)) ? 22 : 21)),
      getBlockHash: async (height: bigint) => codec(hash((Number(height) === 20 && ((state.absent && fault === 'history') || (state.proofReads > 0 && fault === 'late-history'))) ||
        (Number(height) === 21 && fault === 'intermediate-history' && state.checkpointPins.includes(hash(21))) ? 99 : Number(height))),
    },
    rpc: { chain: { getHeader: async (pin: HexString) => ({ number: { toBigInt: () => BigInt(pin) } }) },
      state: { getStorage: async (_key: string, pin: HexString) => ({ isEmpty: false,
        toU8a: () => pin !== profile.sourceBlockHash && fault === 'runtime' ? Uint8Array.of(9) : runtime }) } },
    programStorage: { getProgram: async (id: HexString, pin: HexString) => {
      if (pin !== profile.sourceBlockHash && fault === 'stalled') await new Promise<void>(resolve => { release = resolve; });
      const index = [proxyId, endpointId, checkpointId].indexOf(id);
      return { state: { isInitialized: true }, codeId: codec(hash(pin !== profile.sourceBlockHash && fault === ['proxy-code', 'endpoint-code', 'checkpoint-code'][index] ? 99 : 11 + index)) };
    } },
    message: { calculateReply: async ({ destination, payload, at }: { destination: HexString; payload: Uint8Array; at: HexString }) => {
      const [service, method] = registry.createType('(String,String)', payload);
      let type: string, result: unknown;
      if (method.toString() === 'Network') {
        return { code: { isSuccess: true }, payload: registry.createType('Bytes', registry.createType('(String,String,Network)',
          [service.toString(), method.toString(), fault === 'network' ? 'Mainnet' : 'Hoodi']).toHex()) };
      }
      if (destination === proxyId) { type = 'Result<[u8;32],ProxyError>'; result = { ok: at !== profile.sourceBlockHash && fault === 'endpoint' ? hash(99) : endpointId }; }
      else if (destination === endpointId) { type = '[u8;32]'; result = at !== profile.sourceBlockHash && fault === 'checkpoint' ? hash(99) : checkpointId; }
      else {
        type = 'Result<(u64,H256),CheckpointError>'; state.checkpointPins.push(at);
        if (fault === 'unknown') throw new Error('Unknown checkpoint failure');
        result = at === profile.sourceBlockHash || fault === 'absent' || (at === hash(21) && fault === 'intermediate-history') ? { err: fault === 'outdated' ? 'OutDated' : 'NotPresent' } : { ok: [100, root] };
        state.absent = true;
      }
      return { code: { isSuccess: true }, payload: registry.createType('Bytes', registry.createType('(String,String,' + type + ')', [service.toString(), method.toString(), result]).toHex()) };
    } },
  } as unknown as GearApi;
  const beacon = { genesisBlock: { genesis_time: '0', genesis_validators_root: profile.beaconGenesisValidatorsRoot },
    getBlock: async () => { state.proofReads++; return { ...ssz.fulu.BeaconBlock.toJson(block), fork: 'fulu' }; },
    getSpec: async () => ({ ELECTRA_FORK_EPOCH: '0', ELECTRA_FORK_VERSION: '0x01000000', FULU_FORK_EPOCH: '1', FULU_FORK_VERSION: '0x02000000',
      ...(fault === 'fork' ? { UNKNOWN_FORK_EPOCH: '2', UNKNOWN_FORK_VERSION: '0x03000000' } : {}) }),
    getBlockHeader: async () => ({ canonical: true, root }),
  } as unknown as BeaconClient;
  const ethereum = { getTransactionReceipt: async () => receipt, getSlot: async () => 100,
    getBlockByHash: async () => ({ hash: receipt.blockHash, number: receipt.blockNumber, transactions: [receipt.transactionHash], receiptsRoot }) } as unknown as EthereumClient;
  return { state, profile, beacon, ethereum, proxy: new HistoricalProxyClient(api, proxyId), receipt, receiptsRoot };
}

describe('SDK unsigned checkpoint waiting', () => {
  beforeEach(() => vi.useFakeTimers());
  afterEach(() => vi.useRealTimers());

  test('waits for an applied checkpoint at a later authenticated finalized pin', async () => {
    const fixture = checkpointProofFixture();
    const pending = composeProof(fixture.beacon, fixture.ethereum, fixture.proxy, fixture.receipt.transactionHash, fixture.profile, true).catch(error => error);
    await vi.advanceTimersByTimeAsync(3000);
    const proof = await pending;
    expect(proof).not.toBeInstanceOf(Error);
    expect(proof.proofBlock.block.slot).toBe(100);
    expect(bytesToHex(proof.proofBlock.block.body.executionPayload.receiptsRoot)).toBe(fixture.receiptsRoot);
    expect(fixture.state.checkpointPins).toEqual([hash(20), hash(21)]);
  });

  test.each([
    ['history', 'history'], ['regression', 'regressed'], ['runtime', 'runtime identity'],
    ['intermediate-history', 'history'], ['late-history', 'history'], ['unknown', 'Unknown checkpoint failure'],
    ['endpoint', 'endpointFor'], ['checkpoint', 'checkpoint identity'], ['network', 'immutable network'],
    ['proxy-code', 'CodeId'], ['endpoint-code', 'CodeId'], ['checkpoint-code', 'CodeId'],
    ['outdated', 'OutDated'], ['fork', 'Unreviewed active Beacon fork'],
  ])('holds instead of accepting %s while waiting unsigned', async (fault, expected) => {
    const fixture = checkpointProofFixture(fault);
    const pending = composeProof(fixture.beacon, fixture.ethereum, fixture.proxy, fixture.receipt.transactionHash, fixture.profile, true).catch(error => error);
    await vi.advanceTimersByTimeAsync(6000);
    const error = await pending as Error;
    expect(error).toBeInstanceOf(Error);
    expect(error.message).toContain(expected);
  });

  test('wait:false refuses the absent supplied-pin checkpoint', async () => {
    const fixture = checkpointProofFixture();
    await expect(composeProof(fixture.beacon, fixture.ethereum, fixture.proxy, fixture.receipt.transactionHash, fixture.profile)).rejects.toThrow('Checkpoint');
    expect(fixture.state.proofReads).toBe(0);
  });

  test('NotPresent polling keeps its original deadline and stops after timeout', async () => {
    const fixture = checkpointProofFixture('absent');
    const deadline = Date.now() + 7000;
    const pending = composeProof(fixture.beacon, fixture.ethereum, fixture.proxy, fixture.receipt.transactionHash, fixture.profile, true, undefined, deadline).catch(error => error);
    await vi.advanceTimersByTimeAsync(7000);
    expect((await pending as Error).message).toBe('Original relay deadline expired');
    const observedPins = [...fixture.state.checkpointPins];
    await vi.advanceTimersByTimeAsync(12000);
    expect(fixture.state.checkpointPins).toEqual(observedPins);
    expect(fixture.state.proofReads).toBe(0);
  });

  test('a late source RPC cannot resume proof preparation after the original deadline', async () => {
    const fixture = checkpointProofFixture('stalled');
    const pending = composeProof(fixture.beacon, fixture.ethereum, fixture.proxy, fixture.receipt.transactionHash, fixture.profile, true, undefined, Date.now() + 4000).catch(error => error);
    await vi.advanceTimersByTimeAsync(4000);
    expect((await pending as Error).message).toBe('Original relay deadline expired');
    fixture.state.release();
    await vi.advanceTimersByTimeAsync(6000);
    expect(fixture.state.proofReads).toBe(0);
  });

  test('an expired original deadline cannot begin checkpoint preparation', async () => {
    const fixture = checkpointProofFixture();
    await expect(composeProof(fixture.beacon, fixture.ethereum, fixture.proxy, fixture.receipt.transactionHash, fixture.profile, true, undefined, Date.now() - 1)).rejects.toThrow('Original relay deadline expired');
  });

});


describe('EthToVara', () => {
  let gearApi: GearApi;
  let publicClient: PublicClient;
  let beaconClient: BeaconClient;
  let ethClient: EthereumClient;
  let historicalProxyClient: HistoricalProxyClient;
  let profile: InboundProofProfile;
  let proof: ProofResult;
  let receiptRlp: string, merkleProof: string, encodedEvent: string;

  beforeAll(async () => {
    receiptRlp = fs.readFileSync('test/tmp/receipt_rlp', 'utf8');
    merkleProof = fs.readFileSync('test/tmp/proof', 'utf8');
    encodedEvent = fs.readFileSync('test/tmp/eth_to_vara_scale', 'utf8');
    const serialized = JSON.parse(fs.readFileSync('test/tmp/inbound-profile.json', 'utf8'));
    profile = { ...serialized, ethereumChainId: BigInt(serialized.ethereumChainId), beaconGenesisTime: BigInt(serialized.beaconGenesisTime),
      forks: serialized.forks.map((fork: { epoch: string }) => ({ ...fork, epoch: BigInt(fork.epoch) })) };
    gearApi = await GearApi.create({ providerAddress: process.env.VARA_WS_RPC });
    publicClient = createPublicClient({ transport: webSocket(process.env.ETH_RPC_URL!, { reconnect: false }) });
    beaconClient = await createBeaconClient(process.env.BEACON_RPC_URL!);
    ethClient = createEthereumClient(publicClient, beaconClient);
    historicalProxyClient = new HistoricalProxyClient(gearApi, process.env.HISTORICAL_PROXY_ID! as HexString);
  });
  afterAll(async () => {
    if (gearApi) await gearApi.disconnect();
    if (publicClient) (await publicClient.transport.getRpcClient()).close();
  });

  test('generate proof', async () => {
    proof = await composeProof(beaconClient, ethClient, historicalProxyClient, process.env.TX_HASH as HexString,
      profile);
  });
  test('receipt rlp should be correct', () => expect(bytesToHex(proof.receiptRlp).slice(2)).toEqual(receiptRlp));
  test('proof should be correct', () => expect(proof.proof.map((node) => bytesToHex(node)).map((bytes) => bytes.slice(2)).join('')).toEqual(merkleProof));
  test('eth to vara event should match', () => expect(encodeEthToVaraEvent(proof).slice(2)).toEqual(encodedEvent));
});
