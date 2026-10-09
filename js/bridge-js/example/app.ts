import * as fs from 'node:fs';
import * as path from 'node:path';
import { createHash, randomBytes } from 'node:crypto';
import { setTimeout as delay } from 'node:timers/promises';
import { fileURLToPath } from 'node:url';
import { GearApi, generateCodeHash, generateProgramId } from '@gear-js/api';
import { Keyring } from '@polkadot/keyring';
import { cryptoWaitReady, blake2AsHex } from '@polkadot/util-crypto';
import { Sails } from 'sails-js';
import { SailsIdlParser } from 'sails-js-parser';
import {
  createPublicClient, createWalletClient, http, bytesToHex, hexToBytes, keccak256,
  encodeFunctionData, encodeDeployData, getContractAddress, decodeEventLog, parseTransaction,
  TransactionNotFoundError,
  type Abi, type Address, type Hex, type PublicClient,
} from 'viem';
import { privateKeyToAccount } from 'viem/accounts';
import { hoodi } from 'viem/chains';
import { PingClient } from './lib.js';
import type { Bytes } from '@polkadot/types';
import { immutableInboundProfile, type InboundProofProfile } from '../src/eth-to-vara/proof-composer.js';
import { VFT_MANAGER_IDL_SHA256 } from '../src/eth-to-vara/token-effect.js';

type Direction = 'eth-to-vara' | 'vara-to-eth';
type Mode = 'deploy' | 'send' | 'relay' | 'verify' | 'probe';
type Flags = {
  mode: Mode; intentId: string; sourceIntent?: string; applicationId?: Hex;
  payloadHex?: Hex; resume: boolean; case?: string;
};
type Pin = { number: string; hash: Hex };
type Intent = {
  schemaVersion: 1; runId: string; deploymentDigest: string; intentId: string;
  direction: Direction; operation: Mode; arguments: Omit<Flags, 'resume'>;
  state: 'prepared' | 'broadcast' | 'included' | 'finalized' | 'application-completed' | 'rejected' | 'HOLD';
  deadlineAtMs: string; chain: 'ethereum' | 'gear'; signer: string; nonce: string;
  destination: Hex | null; route: string; calldata: Hex; value: string;
  applicationId?: Hex; payload?: Hex; signedBytes: Hex; hash: Hex;
  preparation: Pin; runtime?: { specVersion: string; transactionVersion: string; codeHash: string; death: string };
  cursor: Pin; replyCursor?: Pin; inclusion?: Pin; requestId?: Hex; replyId?: Hex; replyPin?: Pin;
  result?: unknown; broadcastAtMs?: string;
};
type Run = {
  schemaVersion: number; runId: string; testOnly: boolean;
  bundle: { path: string; sha256: string };
  network: { chainId: number; genesisHash: Hex; executionHttp: string; executionWss: string; beaconHttp: string };
  source: { aliceRpc: string; bobRpc: string };
  runtimeProfile?: RuntimeProfile;
};
type BuildManifest = { schemaVersion: number; testOnly: boolean; runId: string; campaignName: string; files: Record<string, string> };
export type RuntimeProfile = {
  name: 'normal-runtime-hoodi' | 'fast-runtime-hoodi'; runtimeCommit: string; runtimePullRequest: number;
  slotDurationMs: number; epochDurationBlocks: number; warmupDurationMs: number; requiredAuthorityHandovers: number;
  tokenBatchDurationMs: number; applicationAttemptDurationMs: number; testOnly: boolean;
  executionAuthorized: boolean; releaseQualified: boolean; runtimeCiStatus: 'unresolved' | 'failed' | 'passed';
  gearBinarySha256: string; runtimeCodeSha256: string; runtimeCodeBlake2b256: string; runtimeCodeKeccak256: string; approvalSha256: string;
  functionalOnly?: boolean; cadencePatchSha256?: string;
};
export function validateRuntimeProfile(profile: RuntimeProfile, gearBinarySha256: string): void {
  const fast = profile?.name === 'fast-runtime-hoodi';
  check((fast || profile?.name === 'normal-runtime-hoodi') && profile.runtimeCommit === '19ab81dd208b3ce7b339f5d1dbd144511d3c7e2c' &&
    profile.runtimePullRequest === 5642 && profile.slotDurationMs === 3000 && profile.epochDurationBlocks === (fast ? 64 : 2400) &&
    profile.warmupDurationMs === (fast ? 3600000 : 18000000) && profile.requiredAuthorityHandovers === 2 && profile.tokenBatchDurationMs === 3600000 &&
    profile.applicationAttemptDurationMs === MAX_OPERATION_MS && profile.testOnly === true && profile.executionAuthorized === false &&
    profile.releaseQualified === false && ['unresolved', 'failed', 'passed'].includes(profile.runtimeCiStatus), 'HOLD: invalid runtime test profile');
  check((profile.functionalOnly === fast || (!fast && profile.functionalOnly === undefined)) &&
    (fast || !('cadencePatchSha256' in profile)), 'HOLD: fast-only qualification mixed into normal profile');
  if (fast) check(typeof profile.cadencePatchSha256 === 'string' && /^(?:0x)?[0-9a-f]{64}$/.test(profile.cadencePatchSha256) &&
    BigInt('0x' + profile.cadencePatchSha256.replace(/^0x/, '')) !== 0n, 'HOLD: cadence-only patch is not pinned');
  for (const value of [profile.gearBinarySha256, profile.runtimeCodeSha256, profile.runtimeCodeBlake2b256, profile.runtimeCodeKeccak256, profile.approvalSha256]) {
    check(typeof value === 'string' && /^(?:0x)?[0-9a-f]{64}$/.test(value) && BigInt('0x' + value.replace(/^0x/, '')) !== 0n, 'HOLD: missing labeled independently approved artifact hash');
  }
  check(profile.gearBinarySha256.replace(/^0x/, '') === gearBinarySha256?.replace(/^0x/, ''), 'HOLD: selected source binary differs from independent approval');
}
export function applicationAdmission(root: string, run: Run, bundle: { files: Record<string, string>; runtimeProfile?: RuntimeProfile },
  qualification: { runtimeProfile?: RuntimeProfile }, artifacts: string, build: BuildManifest, core: Core, stack: Stack, campaignName: string): InboundProofProfile {
  const runtime = run.runtimeProfile!;
  validateRuntimeProfile(runtime, bundle.files['bin/gear']);
  check(new RegExp('^hoodi-' + (runtime.name === 'fast-runtime-hoodi' ? 'fast' : 'normal') + '-runtime-[a-z0-9][a-z0-9-]{0,42}$').test(campaignName),
    'HOLD: campaign name does not match runtime qualification');
  check(canonical(runtime) === canonical(bundle.runtimeProfile) && canonical(runtime) === canonical(qualification.runtimeProfile), 'HOLD: normal profile differs across sealed owners');
  const approvalPath = path.join(run.bundle.path, 'runtime-approval.json'); noSymlinks(approvalPath);
  const approvalBytes = fs.readFileSync(approvalPath), approval = JSON.parse(approvalBytes.toString()) as { status: string; runtimeProfile: Omit<RuntimeProfile, 'approvalSha256'> };
  const { approvalSha256, ...approvedRuntime } = runtime;
  check(sha256(approvalBytes) === approvalSha256.replace(/^0x/, '') && bundle.files['runtime-approval.json'] === sha256(approvalBytes) &&
    approval.status === 'APPROVED_FOR_TEST_IMPLEMENTATION' && canonical(approval.runtimeProfile) === canonical(approvedRuntime), 'HOLD: independently supplied runtime approval changed');
  const binding = load<{ schemaVersion: number; testOnly: boolean; runId: string; campaignName: string; bundleSha256: string;
    runtimeProfile: RuntimeProfile; automaticRerun: boolean; launched: boolean; preservedFiles: Record<string, string> }>(path.join(root, 'supervisors/normal-campaign-admission.json'));
  check(binding.schemaVersion === 1 && binding.testOnly === true && binding.runId === run.runId && binding.campaignName === campaignName &&
    binding.bundleSha256 === run.bundle.sha256 && canonical(binding.runtimeProfile) === canonical(runtime) && binding.automaticRerun === false && binding.launched === false,
    'HOLD: selected normal campaign admission changed');
  for (const required of ['source-chain/launch-state.json', 'deployment.json', 'token-stack/token-stack.json']) check(binding.preservedFiles[required], 'HOLD: normal admission lacks original identity records');
  for (const [relative, hash] of Object.entries(binding.preservedFiles)) {
    check(relative && !path.isAbsolute(relative) && relative.split('/').every(part => part !== '.' && part !== '..'), 'HOLD: normal admission record escapes owner');
    const filename = path.join(root, relative); noSymlinks(filename);
    check(sha256(fs.readFileSync(filename)) === hash, 'HOLD: original normal admission record changed');
  }
  const launch = load<{ phase: string; identity: Record<string, unknown>; readiness: { genesisHash: Hex };
    pinned: { sourceIdentity: Record<string, unknown> } }>(path.join(root, 'source-chain/launch-state.json'));
  check(launch.phase === 'ready' && canonical(launch.identity.runtimeProfile) === canonical(runtime) &&
    launch.readiness.genesisHash === core.anchor.sourceGenesis && launch.pinned.sourceIdentity.genesisHash === core.anchor.sourceGenesis &&
    core.anchor.sourceGenesis === stack.sourceGenesis, 'HOLD: normal source identity is not independently pinned');
  for (const field of ['runtimeCodeSha256', 'runtimeCodeBlake2b256', 'runtimeCodeKeccak256'] as const) {
    check(String(launch.identity[field]).replace(/^0x/, '') === runtime[field].replace(/^0x/, ''), 'HOLD: source hash algorithm or artifact changed');
  }
  for (const field of ['beefyActivationBlock', 'mmrStartBlock', 'domainBindingBlock', 'bridgeDomain'] as const) {
    check(launch.identity[field] !== undefined && String(launch.identity[field]) === String(launch.pinned.sourceIdentity[field]) &&
      String(launch.identity[field]) === String(core.anchor[field]), 'HOLD: authenticated activation/insertion/domain binding differs');
  }
  const profilePath = path.join(artifacts, 'inbound-proof-profile.json'); noSymlinks(profilePath);
  check(build.files['inbound-proof-profile.json'] && fs.existsSync(profilePath) && sha256(fs.readFileSync(profilePath)) === build.files['inbound-proof-profile.json'],
    'HOLD: independently approved actual actor/profile pins are required, not runtime approval alone');
  const serialized = load<Omit<InboundProofProfile, 'ethereumChainId' | 'beaconGenesisTime' | 'forks'> & {
    ethereumChainId: string; beaconGenesisTime: string; forks: { name: string; epoch: string; version: Hex }[] }>(profilePath);
  const profile = immutableInboundProfile({ ...serialized, ethereumChainId: BigInt(serialized.ethereumChainId), beaconGenesisTime: BigInt(serialized.beaconGenesisTime),
    forks: serialized.forks.map(fork => ({ ...fork, epoch: BigInt(fork.epoch) })) });
  check(profile.ethereumChainId === BigInt(run.network.chainId) && profile.ethereumGenesisHash === run.network.genesisHash &&
    profile.sourceGenesisHash === core.anchor.sourceGenesis &&
    profile.sourceRuntimeCodeHash.slice(2) === runtime.runtimeCodeBlake2b256.replace(/^0x/, '') && profile.beaconGenesisValidatorsRoot === HOODI_BEACON.genesisValidatorsRoot &&
    profile.beaconGenesisTime === 1742213400n && profile.checkpoint.network === 'Hoodi' && profile.historicalProxyId === stack.programs.historicalProxy.id &&
    profile.checkpoint.programId === stack.checkpoint && profile.endpoint.programId === stack.programs.ethEventsElectra.id && profile.consumer.programId === core.gearManager &&
    profile.consumer.service === 'VftManager' && profile.consumer.method === 'SubmitReceipt' && profile.consumer.idlSha256 === VFT_MANAGER_IDL_SHA256 &&
    canonical(profile.forks.map(fork => ({ ...fork, epoch: fork.epoch.toString() }))) === canonical(HOODI_BEACON.forks), 'HOLD: approved normal profile differs from actual selected lane');
  for (const [file, hash] of [['historical_proxy.idl', profile.historicalProxyIdlSha256], ['checkpoint_light_client.idl', profile.checkpoint.idlSha256],
    ['eth_events_electra.idl', profile.endpoint.idlSha256], ['vft_manager.idl', profile.consumer.idlSha256]]) {
    check(build.files[file] === hash, 'HOLD: normal approved deployed IDL differs from qualified application decoder');
  }
  return profile;
}

const CAMPAIGN = process.env.BEEFY_CAMPAIGN_NAME ?? '';
const RUN_ID = 'f991e59c-71cb-4b0a-8c6c-56ce2acb8ac1';
const NAME = /^[A-Za-z0-9][A-Za-z0-9_-]{0,63}$/;
const MAX_OPERATION_MS = 44 * 60_000;
const HOODI_BEACON = { genesisValidatorsRoot: '0x212f13fc4df078b6cb7db228f1c8307566dcecf900867401a92023d7ba99cb5f' as Hex,
  forks: [{ name: 'phase0', epoch: '0', version: '0x10000910' as Hex }, { name: 'altair', epoch: '0', version: '0x20000910' as Hex },
    { name: 'bellatrix', epoch: '0', version: '0x30000910' as Hex }, { name: 'capella', epoch: '0', version: '0x40000910' as Hex },
    { name: 'deneb', epoch: '0', version: '0x50000910' as Hex }, { name: 'electra', epoch: '2048', version: '0x60000910' as Hex }, { name: 'fulu', epoch: '50688', version: '0x70000910' as Hex }] };
const IMPLEMENTATION_SLOT = '0x360894a13ba1a3210667c828492db98dca3e2076cc3735a920a3ca505d382bbc' as const;
const stringify = (value: unknown) => JSON.stringify(value, (_, item) => typeof item === 'bigint' ? item.toString() : item, 2) + '\n';
const sha256 = (value: string | Uint8Array) => createHash('sha256').update(value).digest('hex');
const canonical = (value: unknown): string => JSON.stringify(value, (_, item) => {
  if (typeof item === 'bigint') return item.toString();
  if (item !== null && typeof item === 'object' && !Array.isArray(item)) {
    return Object.fromEntries(Object.entries(item).sort(([a], [b]) => a.localeCompare(b)));
  }
  return item;
});
function check(condition: unknown, message: string): asserts condition {
  if (!condition) throw new Error('HOLD: ' + message);
}
function parseFlags(argv: string[]): Flags {
  const values = new Map<string, string>();
  for (const arg of argv) {
    const match = /^--(mode|intent-id|source-intent|application-id|payload-hex|case)=(.+)$/.exec(arg);
    const key = arg === '--resume' ? 'resume' : match?.[1];
    check(key && !values.has(key), 'unknown, duplicate or malformed flag');
    values.set(key, match?.[2] ?? 'true');
  }
  const mode = values.get('mode') as Mode;
  check(['deploy', 'send', 'relay', 'verify', 'probe'].includes(mode), 'explicit mode is required');
  const intentId = values.get('intent-id');
  check(intentId && NAME.test(intentId), 'invalid intent ID');
  const flags: Flags = { mode, intentId, resume: values.has('resume') };
  const source = values.get('source-intent');
  if (source !== undefined) { check(NAME.test(source), 'invalid source intent'); flags.sourceIntent = source; }
  for (const [key, property] of [['application-id', 'applicationId'], ['payload-hex', 'payloadHex']] as const) {
    const value = values.get(key);
    if (value !== undefined) {
      check(/^0x(?:[0-9a-fA-F]{2})*$/.test(value), 'invalid hex bytes');
      flags[property] = value.toLowerCase() as Hex;
    }
  }
  if (flags.applicationId !== undefined) check(flags.applicationId.length === 66 && BigInt(flags.applicationId) !== 0n, 'application ID must be nonzero bytes32');
  if (flags.payloadHex !== undefined) check(hexToBytes(flags.payloadHex).length <= 1024, 'application payload exceeds 1024 bytes');
  if (mode === 'send') check(flags.applicationId && flags.payloadHex !== undefined && !source, 'send requires application ID and payload only');
  if (mode === 'relay' || mode === 'probe') check(source && !flags.applicationId && flags.payloadHex === undefined, 'relay/probe requires original source intent');
  if (mode === 'deploy' || mode === 'verify') check(!source && !flags.applicationId && flags.payloadHex === undefined, 'deploy/verify does not accept message arguments');
  const probe = values.get('case');
  check((mode === 'probe') === (probe !== undefined), 'case is required only in probe mode');
  if (probe) flags.case = probe;
  check(!(mode === 'verify' && flags.resume), 'verify is read-only, not resume');
  return flags;
}
function noSymlinks(filename: string): void {
  const resolved = path.resolve(filename);
  let current = path.parse(resolved).root;
  for (const part of resolved.slice(current.length).split(path.sep)) {
    current = path.join(current, part);
    try { check(!fs.lstatSync(current).isSymbolicLink(), 'symlink path refused'); }
    catch (error) { if ((error as NodeJS.ErrnoException).code !== 'ENOENT') throw error; }
  }
}
function privateDirectory(directory: string): void {
  noSymlinks(directory);
  if (!fs.existsSync(directory)) fs.mkdirSync(directory, { mode: 0o700 });
  const stat = fs.lstatSync(directory);
  check(stat.isDirectory() && stat.uid === process.getuid?.() && (stat.mode & 0o777) === 0o700, 'unsafe private directory');
}
function privateBytes(filename: string): Buffer {
  noSymlinks(filename);
  const fd = fs.openSync(filename, fs.constants.O_RDONLY | fs.constants.O_NOFOLLOW);
  try {
    const stat = fs.fstatSync(fd);
    check(stat.isFile() && stat.uid === process.getuid?.() && (stat.mode & 0o777) === 0o600, 'unsafe private file');
    return fs.readFileSync(fd);
  } finally { fs.closeSync(fd); }
}
function load<T>(filename: string): T {
  noSymlinks(filename);
  return JSON.parse(fs.readFileSync(filename, 'utf8')) as T;
}
function syncDirectory(directory: string): void {
  const fd = fs.openSync(directory, fs.constants.O_RDONLY);
  try { fs.fsyncSync(fd); } finally { fs.closeSync(fd); }
}
function persist(filename: string, value: unknown, previous?: string): string {
  noSymlinks(filename);
  if (previous !== undefined) check(sha256(privateBytes(filename)) === previous, 'journal changed outside its owner');
  else check(!fs.existsSync(filename), 'original intent already exists; use identical resume');
  const bytes = stringify(value);
  const temporary = filename + '.tmp';
  const fd = fs.openSync(temporary, fs.constants.O_WRONLY | fs.constants.O_CREAT | fs.constants.O_EXCL | fs.constants.O_NOFOLLOW, 0o600);
  try { fs.writeFileSync(fd, bytes); fs.fsyncSync(fd); } finally { fs.closeSync(fd); }
  fs.renameSync(temporary, filename);
  syncDirectory(path.dirname(filename));
  return sha256(bytes);
}
function withinDeadline(intent: Intent): void {
  check(BigInt(Date.now()) < BigInt(intent.deadlineAtMs), 'original 44-minute intent deadline expired');
}
function immutableArguments(flags: Flags): Omit<Flags, 'resume'> {
  const { resume: _, ...result } = flags;
  return result;
}
class Journal {
  private digest: string;
  constructor(readonly filename: string, public intent: Intent, existing = false) {
    this.digest = existing ? sha256(privateBytes(filename)) : persist(filename, intent);
  }
  save(): void { this.digest = persist(this.filename, this.intent, this.digest); }
}
async function sourcePin(api: GearApi, witness: GearApi): Promise<Pin> {
  const source = await api.rpc.chain.getHeader(await api.rpc.chain.getFinalizedHead());
  const witnessFinal = await witness.rpc.chain.getHeader(await witness.rpc.chain.getFinalizedHead());
  const height = BigInt(source.number.toString()) < BigInt(witnessFinal.number.toString()) ? source.number.toString() : witnessFinal.number.toString();
  const hash = (await api.rpc.chain.getBlockHash(height)).toHex();
  check((await witness.rpc.chain.getBlockHash(height)).toHex() === hash, 'source/witness finalized disagreement');
  return { number: height, hash };
}
async function assertSourcePin(api: GearApi, witness: GearApi, pin: Pin): Promise<void> {
  const finalized = await sourcePin(api, witness);
  check(BigInt(finalized.number) >= BigInt(pin.number), 'source finalized height regressed');
  check((await api.rpc.chain.getBlockHash(pin.number)).toHex() === pin.hash && (await witness.rpc.chain.getBlockHash(pin.number)).toHex() === pin.hash, 'original source history changed');
}
async function sails(api: GearApi, idl: string, programId: Hex): Promise<Sails> {
  return new Sails(await SailsIdlParser.new()).parseIdl(idl).setApi(api).setProgramId(programId);
}
async function scanGear(journal: Journal, api: GearApi, witness: GearApi): Promise<void> {
  const intent = journal.intent;
  await assertSourcePin(api, witness, intent.cursor);
  if (intent.inclusion) { await assertSourcePin(api, witness, intent.inclusion); return; }
  const final = await sourcePin(api, witness);
  for (let height = BigInt(intent.cursor.number) + 1n; height <= BigInt(final.number); height++) {
    withinDeadline(intent);
    const hash = (await api.rpc.chain.getBlockHash(height.toString())).toHex();
    check((await witness.rpc.chain.getBlockHash(height.toString())).toHex() === hash, 'source scan witness disagreement');
    const block = await api.rpc.chain.getBlock(hash);
    const index = block.block.extrinsics.findIndex(tx => tx.hash.toHex() === intent.hash);
    if (index >= 0) {
      const tx = block.block.extrinsics[index];
      check(tx.toHex() === intent.signedBytes && tx.nonce.toString() === intent.nonce, 'original extrinsic changed');
      const events = await api.query.system.events.at(hash);
      const own = events.filter(e => e.phase.isApplyExtrinsic && e.phase.asApplyExtrinsic.toNumber() === index);
      check(own.some(({ event }) => event.section === 'system' && event.method === 'ExtrinsicSuccess'), 'original extrinsic failed dispatch');
      const queued = own.filter(({ event }) => event.section === 'gear' && event.method === 'MessageQueued');
      check(queued.length === 1, 'original extrinsic has ambiguous request identity');
      const data = queued[0].event.data as unknown as { id: { toHex(): Hex }; source: { toHex(): Hex }; destination: { toHex(): Hex } };
      check(data.source.toHex() === intent.signer && data.destination.toHex() === intent.destination, 'original request route mismatch');
      intent.requestId = data.id.toHex(); intent.inclusion = { number: height.toString(), hash }; intent.state = 'finalized';
    } else {
      for (const tx of block.block.extrinsics) {
        if (tx.isSigned && tx.nonce.toString() === intent.nonce && api.createType('AccountId', tx.signer.toString()).toHex() === intent.signer) check(false, 'original Gear nonce consumed by conflicting transaction');
      }
    }
    intent.cursor = { number: height.toString(), hash }; journal.save();
    if (intent.inclusion) return;
  }
}
async function broadcastGear(journal: Journal, api: GearApi, witness: GearApi): Promise<void> {
  const intent = journal.intent;
  while (true) {
    withinDeadline(intent);
    await scanGear(journal, api, witness);
    if (intent.inclusion) return;
    const pin = await sourcePin(api, witness);
    check(intent.runtime && BigInt(pin.number) < BigInt(intent.runtime.death), 'original Gear mortality expired without proven outcome');
    const at = await api.at(pin.hash);
    check((await at.query.system.account(intent.signer)).nonce.toString() === intent.nonce, 'Gear finalized nonce unavailable without original outcome');
    const pending = await api.rpc.author.pendingExtrinsics();
    const original = pending.find(tx => tx.hash.toHex() === intent.hash);
    const next = BigInt((await api.rpc.system.accountNextIndex(intent.signer)).toString());
    check(next <= BigInt(intent.nonce) + 1n, 'Gear signer has other outstanding nonces');
    if (original) check(original.toHex() === intent.signedBytes, 'original pending Gear bytes changed');
    else if (next === BigInt(intent.nonce)) {
      check(api.tx(intent.signedBytes).hash.toHex() === intent.hash, 'persisted signed Gear hash mismatch');
      withinDeadline(intent);
      intent.state = 'broadcast'; intent.broadcastAtMs ??= String(Date.now()); journal.save();
      const hash = await api.rpc.author.submitExtrinsic(intent.signedBytes);
      check(hash.toHex() === intent.hash, 'Gear broadcast returned a different hash');
    }
    // An included, not-yet-finalized original is absent from the pool. Scan to finality; never replace it.
    await delay(3000);
  }
}
async function broadcastEthereum(journal: Journal, client: PublicClient): Promise<void> {
  const intent = journal.intent;
  while (true) {
    withinDeadline(intent);
    const final = await client.getBlock({ blockTag: 'finalized' });
    check(final.number !== null && final.hash, 'Ethereum finalized block unavailable');
    check((await client.getBlock({ blockNumber: BigInt(intent.cursor.number) })).hash === intent.cursor.hash, 'original EVM scan cursor changed');
    if (!intent.inclusion) {
      for (let height = BigInt(intent.cursor.number) + 1n; height <= final.number; height++) {
        withinDeadline(intent);
        const block = await client.getBlock({ blockNumber: height, includeTransactions: true });
        for (const tx of block.transactions) {
          if (tx.from.toLowerCase() !== intent.signer.toLowerCase() || String(tx.nonce) !== intent.nonce) continue;
          check(tx.hash === intent.hash && tx.input === intent.calldata && tx.value.toString() === intent.value && (tx.to?.toLowerCase() ?? null) === (intent.destination?.toLowerCase() ?? null), 'original EVM nonce consumed by conflicting transaction');
          intent.inclusion = { number: height.toString(), hash: block.hash! }; intent.state = 'finalized';
        }
        intent.cursor = { number: height.toString(), hash: block.hash! }; journal.save();
        if (intent.inclusion) break;
      }
    }
    if (intent.inclusion) {
      check((await client.getBlock({ blockNumber: BigInt(intent.inclusion.number) })).hash === intent.inclusion.hash, 'original EVM inclusion changed');
      return;
    }
    check(BigInt(await client.getTransactionCount({ address: intent.signer as Address, blockTag: 'finalized' })) <= BigInt(intent.nonce), 'finalized nonce consumed without original evidence');
    const latestNonce = await client.getTransactionCount({ address: intent.signer as Address, blockTag: 'latest' });
    const pendingNonce = await client.getTransactionCount({ address: intent.signer as Address, blockTag: 'pending' });
    check(BigInt(pendingNonce) <= BigInt(intent.nonce) + 1n, 'campaign signer has another outstanding nonce');
    let original: Awaited<ReturnType<PublicClient['getTransaction']>> | undefined;
    try { original = await client.getTransaction({ hash: intent.hash }); }
    catch (error) { if (!(error instanceof TransactionNotFoundError)) throw error; }
    if (original) check(original.hash === intent.hash && original.from.toLowerCase() === intent.signer.toLowerCase() && String(original.nonce) === intent.nonce && original.input === intent.calldata, 'original pending EVM transaction changed');
    else {
      check(BigInt(latestNonce) === BigInt(intent.nonce) && BigInt(pendingNonce) === BigInt(intent.nonce), 'original EVM nonce has an unresolved conflicting consumer');
      check(keccak256(intent.signedBytes) === intent.hash, 'persisted signed EVM hash mismatch');
      withinDeadline(intent);
      intent.state = 'broadcast'; intent.broadcastAtMs ??= String(Date.now()); journal.save();
      const hash = await client.sendRawTransaction({ serializedTransaction: intent.signedBytes });
      check(hash === intent.hash, 'EVM broadcast returned a different hash');
    }
    await delay(12_000);
  }
}

type Core = {
  anchor: { sourceGenesis: Hex; bridgeDomain: Hex; beefyActivationBlock: number; mmrStartBlock: number; domainBindingBlock: number };
  ethereum: { queue: Address; client: Address; verifier: Address; bytecodeHashes: Record<string, Hex> };
  ethereumConfiguration: { tokens: { address: Address; symbol: string; decimals: number }[] };
  gearManager: Hex;
};
type Stack = { checkpoint: Hex; sourceGenesis: Hex; programs: Record<string, { id: Hex }> };
type BridgeConfig = { builtin: Hex; fee_bridge: string; gas_to_send_request_to_builtin: string; gas_for_reply_deposit: string; reply_timeout: number };
type Preparation = {
  schemaVersion: 1; testOnly: true; runId: string; campaignName: string; intentId: string; deadlineAtMs: string;
  descriptorHashes: Record<string, string>; buildManifestDigest: string; source: Pin; ethereum: Pin;
  sourceRuntimeSha256: string; sourceRuntimeCodeHash: Hex; sourceGenesis: Hex; bridgeDomain: Hex; queue: Address; queueImplementation: Address; client: Address; verifier: Address;
  proxy: Hex; checkpoint: Hex; programCodeIds: Record<string, Hex>; evmCodeHashes: Record<string, Hex>; beacon: typeof HOODI_BEACON;
  endpoint: { programId: Hex; codeId: Hex; idlSha256: string }; checkpointIdlSha256: string;
  caller: { ethereum: Address; gear: Hex }; ping: { salt: Hex; codeId: Hex; programId: Hex; wasmSha256: string; config: BridgeConfig };
  handler: { address: Address; nonce: string; artifactSha256: string; constructor: [Address, Hex, Address] };
};
type Deployment = Preparation & {
  preparationDigest: string; deploymentIntents: { ethereum: { hash: Hex; pin: Pin }; gear: { hash: Hex; pin: Pin; requestId: Hex; replyId: Hex; replyPin: Pin } };
  handlerRuntimeHash: Hex;
};
type Context = {
  root: string; app: string; artifacts: string; flags: Flags; direction: Direction; run: Run; core: Core; stack: Stack;
  build: BuildManifest; client: PublicClient; gear: GearApi; witness: GearApi; handlerAbi: Abi; handlerBytecode: Hex;
  pingIdl: string; managerIdl: string; deployment?: Deployment; binding: string;
  approvedProfile?: InboundProofProfile;
};
async function canonicalEthereumReceipt(ctx: Context, intent: Intent) {
  check(intent.chain === 'ethereum' && intent.inclusion, 'original EVM inclusion missing');
  const [receipt, transaction, block, final] = await Promise.all([
    ctx.client.getTransactionReceipt({ hash: intent.hash }), ctx.client.getTransaction({ hash: intent.hash }),
    ctx.client.getBlock({ blockNumber: BigInt(intent.inclusion.number) }), ctx.client.getBlock({ blockTag: 'finalized' }),
  ]);
  check(receipt.transactionHash === intent.hash && transaction.hash === intent.hash && block.hash === intent.inclusion.hash
    && receipt.blockHash === block.hash && transaction.blockHash === block.hash && receipt.blockNumber === block.number
    && transaction.transactionIndex === receipt.transactionIndex && final.number! >= receipt.blockNumber, 'original EVM receipt is not canonical and finalized');
  check(transaction.from.toLowerCase() === intent.signer.toLowerCase() && String(transaction.nonce) === intent.nonce
    && transaction.input === intent.calldata && transaction.value.toString() === intent.value
    && (transaction.to?.toLowerCase() ?? null) === (intent.destination?.toLowerCase() ?? null), 'original EVM transaction fields changed');
  return receipt;
}
async function connect(direction: Direction, flags: Flags): Promise<Context> {
  check(process.env.BEEFY_RUN, 'BEEFY_RUN missing');
  check(CAMPAIGN && NAME.test(CAMPAIGN), 'selected verified campaign is required');
  const root = path.resolve(process.env.BEEFY_RUN); noSymlinks(root);
  const run = load<Run>(path.join(root, 'run.json'));
  const normal = run.runtimeProfile !== undefined;
  check(run.schemaVersion === 1 && run.testOnly && path.basename(root) === run.runId && (normal ?
    run.runId !== RUN_ID && /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/.test(run.runId) &&
    /^hoodi-(?:normal|fast)-runtime-[a-z0-9][a-z0-9-]{0,42}$/.test(CAMPAIGN) : run.runId === RUN_ID), 'wrong retained or distinct profiled test lane');
  check(run.network.chainId === 560048, 'Hoodi only');
  const core = load<Core>(path.join(root, 'deployment.json'));
  const stack = load<Stack>(path.join(root, 'token-stack/token-stack.json'));
  const app = path.join(root, 'app-messages');
  const bundlePath = path.join(run.bundle.path, 'bundle.json');
  const bundle = load<{ files: Record<string, string>; runtimeProfile?: RuntimeProfile }>(bundlePath);
  check(sha256(fs.readFileSync(bundlePath)) === run.bundle.sha256, 'selected bundle manifest changed');
  const qualificationPath = path.join(run.bundle.path, 'verification.json');
  const qualification = load<{ campaignName: string; runtimeProfile?: RuntimeProfile; applicationArtifacts?: { path: string; manifestSha256: string } }>(qualificationPath);
  check(qualification.campaignName === CAMPAIGN, 'selected campaign differs from sealed verification');
  check(sha256(fs.readFileSync(qualificationPath)) === bundle.files['verification.json'], 'selected application qualification changed');
  let application = qualification.applicationArtifacts;
  if (normal) {
    check(!('applicationArtifacts' in qualification), 'HOLD: profiled application artifacts require post-deployment qualification');
    const filename = path.join(root, 'qualification', CAMPAIGN + '-applications.json'); noSymlinks(filename);
    check(fs.lstatSync(filename).isFile() && !(fs.statSync(filename).mode & 0o222), 'HOLD: application qualification is not immutable');
    const selected = load<{ schemaVersion: number; testOnly: boolean; status: string; runId: string; campaignName: string; runtimeProfile: RuntimeProfile;
      componentBundleSha256: string; componentVerificationSha256: string; applicationArtifacts: { path: string; manifestSha256: string };
      checks: { command: string; exitCode: number; logPath: string; logSha256: string }[] }>(filename);
    check(selected.schemaVersion === 1 && selected.testOnly === true && selected.status === 'VERIFIED' && selected.runId === run.runId &&
      selected.campaignName === CAMPAIGN && canonical(selected.runtimeProfile) === canonical(run.runtimeProfile) &&
      selected.componentBundleSha256 === run.bundle.sha256 && selected.componentVerificationSha256 === bundle.files['verification.json'],
      'HOLD: application qualification differs from the original lane');
    const required = ['cargo build --locked -p ping --release', 'forge build --root js/bridge-js/js-test/contracts --force --no-cache',
      'forge test --root js/bridge-js/js-test/contracts --match-contract MessageHandlerTest -vvv', 'yarn workspace @gear-js/bridge typecheck',
      'yarn workspace @gear-js/bridge test test/vara-to-eth.test.ts', 'yarn workspace @gear-js/bridge test test/eth-to-vara.test.ts',
      'yarn workspace @gear-js/bridge build:examples'];
    check(required.every(command => selected.checks.some(check => check.command === command && check.exitCode === 0)), 'HOLD: full owned application checks are missing');
    for (const result of selected.checks) {
      const relative = path.relative(path.join(root, 'qualification'), result.logPath);
      check(result.exitCode === 0 && result.command && path.isAbsolute(result.logPath) && path.resolve(result.logPath) === result.logPath &&
        relative && relative !== '..' && !relative.startsWith('../') && !path.isAbsolute(relative), 'HOLD: application check failed or escaped its owner');
      noSymlinks(result.logPath);
      check(fs.lstatSync(result.logPath).isFile() && !(fs.statSync(result.logPath).mode & 0o222) && sha256(fs.readFileSync(result.logPath)) === result.logSha256,
        'HOLD: immutable application check evidence changed');
    }
    application = selected.applicationArtifacts;
  }
  check(application, 'HOLD: selected application artifacts are missing');
  const artifacts = application.path;
  check(path.isAbsolute(artifacts) && path.resolve(artifacts) === artifacts && path.dirname(artifacts) === app, 'application artifact directory escapes its owner');
  const buildPath = path.join(artifacts, 'manifest.json');
  const build = load<BuildManifest>(buildPath);
  check(sha256(fs.readFileSync(buildPath)) === application.manifestSha256, 'application manifest differs from qualification');
  check(build.schemaVersion === 1 && build.testOnly && build.runId === run.runId && build.campaignName === CAMPAIGN, 'unqualified application artifacts');
  for (const [relative, hash] of Object.entries(build.files)) {
    check(relative !== '' && !path.isAbsolute(relative) && relative.split('/').every(x => x !== '.' && x !== '..'), 'artifact path escapes manifest');
    const filename = path.join(artifacts, relative); noSymlinks(filename);
    check(sha256(fs.readFileSync(filename)) === hash, 'application artifact digest mismatch');
  }
  check(path.resolve(fileURLToPath(import.meta.url)) === path.join(artifacts, 'lib/demo/example/app.js'), 'execute only hash-pinned compiled examples');
  check(build.files['lib/demo/example/app.js'], 'executing application not in artifact manifest');
  check(normal || (bundle.runtimeProfile === undefined && qualification.runtimeProfile === undefined && /^hoodi-milestone-1(?:-retry-[1-9][0-9]*)?$/.test(CAMPAIGN)), 'HOLD: profile owner mismatch or normal campaign on retained lane');
  const approvedProfile = normal ? applicationAdmission(root, run, bundle, qualification, artifacts, build, core, stack, CAMPAIGN) : undefined;
  check(process.execPath === path.join(artifacts, 'bin/node') && build.files['bin/node'], 'execute only the hash-pinned Node runtime');
  if (flags.mode !== 'verify') {
    const campaign = load<{ preflight: { status: string }; warmup: { status: string } }>(path.join(root, 'campaigns', CAMPAIGN, 'campaign-state.json'));
    check(campaign.preflight.status === 'passed' && campaign.warmup.status === 'passed', 'token preflight and warmup have not passed');
    const fd = process.env.BEEFY_CAMPAIGN_LOCK_FD;
    check(fd && /^(0|[1-9][0-9]*)$/.test(fd), 'existing admission wrapper lock required');
    const held = fs.fstatSync(Number(fd)), expected = fs.statSync(path.join(root, 'bounded-campaign.lock'));
    check(held.dev === expected.dev && held.ino === expected.ino, 'wrong inherited deployment-wide lock');
    privateDirectory(app); privateDirectory(path.join(app, 'intents')); privateDirectory(path.join(app, 'results'));
  }
  const handlerArtifact = load<{ abi: Abi; bytecode: { object: Hex } }>(path.join(artifacts, 'MessageHandler.json'));
  const client: PublicClient = createPublicClient({ chain: hoodi, transport: http(run.network.executionHttp, { timeout: 20_000, retryCount: 0 }) });
  check(await client.getChainId() === 560048 && (await client.getBlock({ blockNumber: 0n })).hash === run.network.genesisHash, 'EL chain/genesis mismatch');
  const gear = await GearApi.create({ providerAddress: run.source.aliceRpc, noInitWarn: true });
  let witness: GearApi | undefined;
  try {
    witness = await GearApi.create({ providerAddress: run.source.bobRpc, noInitWarn: true });
    check(gear.genesisHash.toHex() === core.anchor.sourceGenesis && witness.genesisHash.toHex() === core.anchor.sourceGenesis && core.anchor.sourceGenesis === stack.sourceGenesis, 'source genesis mismatch');
    if (approvedProfile) {
      const header = await gear.rpc.chain.getHeader(approvedProfile.sourceBlockHash);
      await assertSourcePin(gear, witness, { number: header.number.toString(), hash: approvedProfile.sourceBlockHash });
    }
    const genesisResponse = await fetch(run.network.beaconHttp + '/eth/v1/beacon/genesis', { signal: AbortSignal.timeout(20_000) });
    check(genesisResponse.ok, 'Beacon transport unavailable');
    const beaconGenesis = await genesisResponse.json() as { data: { genesis_time: string; genesis_validators_root: Hex } };
    check(beaconGenesis.data.genesis_validators_root === HOODI_BEACON.genesisValidatorsRoot && beaconGenesis.data.genesis_time === '1742213400', 'Beacon network/genesis mismatch');
    const specResponse = await fetch(run.network.beaconHttp + '/eth/v1/config/spec', { signal: AbortSignal.timeout(20_000) });
    check(specResponse.ok, 'Beacon fork configuration unavailable');
    const spec = (await specResponse.json() as { data: Record<string, string> }).data;
    check(spec.CONFIG_NAME === 'hoodi' && spec.SECONDS_PER_SLOT === '12' && spec.SLOTS_PER_EPOCH === '32' && HOODI_BEACON.forks.every(f => f.name === 'phase0' ? spec.GENESIS_FORK_VERSION === f.version : spec[f.name.toUpperCase() + '_FORK_EPOCH'] === f.epoch && spec[f.name.toUpperCase() + '_FORK_VERSION'] === f.version), 'unqualified Beacon fork schedule');
    const deploymentPath = path.join(app, 'deployment.json');
    const deployment = fs.existsSync(deploymentPath) ? load<Deployment>(deploymentPath) : undefined;
    if (deployment) {
      check(deployment.runId === run.runId && deployment.campaignName === CAMPAIGN && deployment.testOnly === true, 'application deployment identity mismatch');
      for (const [file, hash] of Object.entries(deployment.descriptorHashes)) check(sha256(fs.readFileSync(path.join(root, file))) === hash, 'retained descriptor changed');
      check(deployment.buildManifestDigest === sha256(fs.readFileSync(path.join(artifacts, 'manifest.json'))), 'application build changed');
      check(deployment.queue === core.ethereum.queue && deployment.proxy === stack.programs.historicalProxy.id && deployment.checkpoint === stack.checkpoint, 'application lane changed');
    }
    return { root, app, artifacts, flags, direction, run, core, stack, build, client, gear, witness, approvedProfile,
      handlerAbi: handlerArtifact.abi, handlerBytecode: handlerArtifact.bytecode.object,
      pingIdl: fs.readFileSync(path.join(artifacts, 'ping.idl'), 'utf8'), managerIdl: fs.readFileSync(path.join(artifacts, 'vft_manager.idl'), 'utf8'),
      deployment, binding: deployment ? sha256(privateBytes(deploymentPath)) : '' };
  } catch (error) { await gear.disconnect(); await witness?.disconnect(); throw error; }
}
async function programCodeId(api: GearApi, pin: Pin, id: Hex): Promise<Hex> {
  const at = await api.at(pin.hash);
  const program = (await at.query.gearProgram.programStorage(id)).toJSON() as { active?: { codeId: Hex; state: { initialized?: null } } } | null;
  check(program?.active && 'initialized' in program.active.state, 'program is absent or uninitialized');
  return program.active.codeId;
}
async function actualBridgeConfig(ctx: Context, pin: Pin): Promise<BridgeConfig> {
  await assertSourcePin(ctx.gear, ctx.witness, pin);
  const manager = await sails(ctx.gear, ctx.managerIdl, ctx.core.gearManager);
  const config = await manager.services.VftManager.queries.GetConfig().atBlock(pin.hash).call() as Record<string, string | number>;
  const builtin = await manager.services.VftManager.queries.GearBridgeBuiltin().atBlock(pin.hash).call() as Hex;
  const at = await ctx.gear.at(pin.hash);
  const fee = (await at.query.gearEthBridge.transportFee()).toString();
  check(String(config.fee_bridge) === fee && String(config.gas_to_send_request_to_builtin) === '10000000000'
    && String(config.gas_for_reply_deposit) === '10000000000' && String(config.reply_timeout) === '100' && /^0x[0-9a-f]{64}$/.test(builtin) && BigInt(builtin) !== 0n, 'retained builtin/config drift');
  return { builtin, fee_bridge: fee, gas_to_send_request_to_builtin: String(config.gas_to_send_request_to_builtin), gas_for_reply_deposit: String(config.gas_for_reply_deposit), reply_timeout: 100 };
}
async function authenticateDeployment(ctx: Context): Promise<void> {
  const deployment = ctx.deployment; check(deployment, 'finalized application deployment required');
  const pin = await sourcePin(ctx.gear, ctx.witness);
  const runtime = await ctx.gear.rpc.state.getStorage<Bytes>(':code', pin.hash);
  check(sha256(runtime.toU8a(true)) === deployment.sourceRuntimeSha256, 'source runtime drift');
  check(canonical(await actualBridgeConfig(ctx, pin)) === canonical(deployment.ping.config), 'immutable fee/config drift');
  for (const [id, code] of Object.entries(deployment.programCodeIds)) check(await programCodeId(ctx.gear, pin, id as Hex) === code, 'retained program code changed');
  check(await programCodeId(ctx.gear, pin, deployment.ping.programId) === deployment.ping.codeId, 'Ping code changed');
  const final = await ctx.client.getBlock({ blockTag: 'finalized' });
  for (const [address, hash] of Object.entries(deployment.evmCodeHashes)) check(keccak256(await ctx.client.getCode({ address: address as Address, blockNumber: final.number! }) ?? '0x') === hash, 'retained EVM runtime code changed');
  check(keccak256(await ctx.client.getCode({ address: deployment.handler.address, blockNumber: final.number! }) ?? '0x') === deployment.handlerRuntimeHash, 'MessageHandler runtime code changed');
  check((await ctx.client.getStorageAt({ address: deployment.queue, slot: IMPLEMENTATION_SLOT, blockNumber: final.number! }))?.slice(-40).toLowerCase() === deployment.queueImplementation.slice(2).toLowerCase(), 'queue implementation pointer changed');
  const queueAbi = load<{ abi: Abi }>(path.join(ctx.artifacts, 'MessageQueue.json')).abi;
  check(String(await ctx.client.readContract({ address: deployment.queue, abi: queueAbi, functionName: 'verifier', blockNumber: final.number! })).toLowerCase() === deployment.verifier.toLowerCase(), 'queue verifier pointer changed');
}
function originalIntent(ctx: Context, flags = ctx.flags): Journal | undefined {
  const filename = path.join(ctx.app, 'intents', flags.intentId + '.json');
  if (!fs.existsSync(filename)) { check(!flags.resume && flags.mode !== 'verify', 'original intent missing'); return; }
  check(flags.resume || flags.mode === 'verify', 'original intent exists; use identical --resume');
  const intent = JSON.parse(privateBytes(filename).toString()) as Intent;
  check(intent.schemaVersion === 1 && intent.runId === ctx.run.runId && intent.direction === ctx.direction && intent.intentId === flags.intentId && intent.deploymentDigest === ctx.binding, 'original intent identity mismatch');
  if (flags.mode !== 'verify') check(canonical(intent.arguments) === canonical(immutableArguments(flags)), 'resume arguments changed');
  const caller = ctx.deployment?.caller ?? load<Preparation>(path.join(ctx.app, 'preparation.json')).caller;
  check(intent.signer.toLowerCase() === (intent.chain === 'ethereum' ? caller.ethereum : caller.gear).toLowerCase(), 'original signer differs from retained campaign caller');
  check(intent.signedBytes !== '0x' && (intent.chain === 'ethereum' ? keccak256(intent.signedBytes) : ctx.gear.tx(intent.signedBytes).hash.toHex()) === intent.hash, 'original signed-byte digest mismatch');
  if (intent.chain === 'ethereum') {
    const tx = parseTransaction(intent.signedBytes);
    check(tx.chainId === 560048 && String(tx.nonce) === intent.nonce && (tx.to?.toLowerCase() ?? null) === (intent.destination?.toLowerCase() ?? null) && tx.data === intent.calldata && String(tx.value ?? 0n) === intent.value, 'original signed EVM intent fields mismatch');
  } else {
    const tx = ctx.gear.tx(intent.signedBytes);
    check(ctx.gear.createType('AccountId', tx.signer.toString()).toHex() === intent.signer && tx.nonce.toString() === intent.nonce && tx.args[intent.route === 'New' ? 2 : 1].toHex() === intent.calldata, 'original signed Gear intent fields mismatch');
  }
  return new Journal(filename, intent, true);
}
function credentials(ctx: Context) {
  check(ctx.flags.mode !== 'verify', 'verify never opens signers');
  check(process.env.ETH_CAMPAIGN_KEY_FILE === path.join(ctx.root, 'hoodi/keys/campaign.key') && process.env.GEAR_CAMPAIGN_SURI_FILE === path.join(ctx.root, 'hoodi/gear-keys/campaign.suri'), 'only retained idle campaign signers are authorized');
  const ethereum = privateKeyToAccount(privateBytes(process.env.ETH_CAMPAIGN_KEY_FILE!).toString().trim() as Hex);
  const gear = new Keyring({ type: 'sr25519', ss58Format: 137 }).createFromUri(privateBytes(process.env.GEAR_CAMPAIGN_SURI_FILE!).toString().trim());
  const evmAddress = load<{ roles: { campaign: string } }>(path.join(ctx.root, 'hoodi/addresses.json')).roles.campaign;
  const gearAddress = load<{ roles: { campaign: { publicKey: string } } }>(path.join(ctx.root, 'hoodi/gear-addresses.json')).roles.campaign.publicKey;
  check(ethereum.address.toLowerCase() === evmAddress.toLowerCase() && bytesToHex(gear.publicKey) === gearAddress, 'campaign signer identity mismatch');
  return { ethereum, gear };
}
async function signEthereum(ctx: Context, calldata: Hex, destination: Address | null, route: string, deadline: string, value = 0n, flags = ctx.flags, result?: RelayEvidence): Promise<Journal> {
  const accounts = credentials(ctx), final = await ctx.client.getBlock({ blockTag: 'finalized' });
  const nonce = await ctx.client.getTransactionCount({ address: accounts.ethereum.address, blockTag: 'pending' });
  check(nonce === await ctx.client.getTransactionCount({ address: accounts.ethereum.address, blockTag: 'latest' }), 'campaign signer has pending EVM intent');
  const wallet = createWalletClient({ account: accounts.ethereum, chain: hoodi, transport: http(ctx.run.network.executionHttp) });
  const request = await wallet.prepareTransactionRequest({ account: accounts.ethereum, chain: hoodi, type: 'eip1559', to: destination ?? undefined, data: calldata, value, nonce,
    ...(flags.mode === 'probe' ? { gas: 2_000_000n } : {}) });
  check(request.gas !== undefined && request.gas <= final.gasLimit, 'EVM demand exceeds authentic block gas limit');
  const balance = await ctx.client.getBalance({ address: accounts.ethereum.address, blockTag: 'pending' });
  check(balance >= value + request.gas * (request.maxFeePerGas ?? request.gasPrice ?? 0n), 'campaign EVM gas funds insufficient; no refill');
  const signedBytes = await accounts.ethereum.signTransaction({ chainId: 560048, type: 'eip1559', to: destination ?? undefined, data: calldata, value, nonce, gas: request.gas, maxFeePerGas: request.maxFeePerGas, maxPriorityFeePerGas: request.maxPriorityFeePerGas });
  const pin: Pin = { number: final.number!.toString(), hash: final.hash! };
  const intent: Intent = { schemaVersion: 1, runId: ctx.run.runId, deploymentDigest: ctx.binding, intentId: flags.intentId, direction: ctx.direction,
    operation: flags.mode, arguments: immutableArguments(flags), state: 'prepared', deadlineAtMs: deadline, chain: 'ethereum', signer: accounts.ethereum.address.toLowerCase(),
    nonce: String(nonce), destination, route, calldata, value: value.toString(), applicationId: result?.applicationId ?? flags.applicationId, payload: result?.payload ?? flags.payloadHex,
    signedBytes, hash: keccak256(signedBytes), preparation: pin, cursor: pin, result };
  return new Journal(path.join(ctx.app, 'intents', flags.intentId + '.json'), intent);
}
async function signGear(ctx: Context, extrinsic: ReturnType<GearApi['message']['send']>, destination: Hex, route: string, payload: Hex, value: bigint, deadline: string, flags = ctx.flags, result?: RelayEvidence): Promise<Journal> {
  const accounts = credentials(ctx), pin = await sourcePin(ctx.gear, ctx.witness), at = await ctx.gear.at(pin.hash);
  const info = await at.query.system.account(accounts.gear.address), nonce = info.nonce.toString();
  check((await ctx.gear.rpc.system.accountNextIndex(accounts.gear.address)).toString() === nonce, 'campaign Gear signer has pending intent');
  const gasLimit = ctx.gear.blockGasLimit.toBigInt() / 100n * 95n;
  const estimated = route === 'New'
    ? await ctx.gear.program.calculateGas.initUpload(bytesToHex(accounts.gear.publicKey), fs.readFileSync(path.join(ctx.artifacts, 'ping.opt.wasm')), payload, value, true)
    : await ctx.gear.program.calculateGas.handle(bytesToHex(accounts.gear.publicKey), destination, payload, value, true);
  check(estimated.min_limit.toBigInt() <= gasLimit, 'estimated demand exceeds retained 95-percent outer gas rule');
  const fee = await extrinsic.paymentInfo(accounts.gear);
  check(info.data.free.toBigInt() >= value + fee.partialFee.toBigInt() + gasLimit * BigInt(ctx.gear.valuePerGas.toString()), 'campaign Gear funds insufficient; no refill');
  const version = await ctx.gear.rpc.state.getRuntimeVersion(pin.hash), code = await ctx.gear.rpc.state.getStorage<import('@polkadot/types').Bytes>(':code', pin.hash);
  check(version.specVersion.eq(ctx.gear.runtimeVersion.specVersion) && version.transactionVersion.eq(ctx.gear.runtimeVersion.transactionVersion), 'runtime changed since preparation');
  const era = ctx.gear.createType('ExtrinsicEra', { current: Number(pin.number), period: 4096 });
  await extrinsic.signAsync(accounts.gear, { nonce, era, blockHash: pin.hash });
  const intent: Intent = { schemaVersion: 1, runId: ctx.run.runId, deploymentDigest: ctx.binding, intentId: flags.intentId, direction: ctx.direction,
    operation: flags.mode, arguments: immutableArguments(flags), state: 'prepared', deadlineAtMs: deadline, chain: 'gear', signer: bytesToHex(accounts.gear.publicKey), nonce,
    destination, route, calldata: payload, value: value.toString(), applicationId: result?.applicationId ?? flags.applicationId, payload: result?.payload ?? flags.payloadHex,
    signedBytes: extrinsic.toHex(), hash: extrinsic.hash.toHex(), preparation: pin, cursor: pin, result,
    runtime: { specVersion: version.specVersion.toString(), transactionVersion: version.transactionVersion.toString(), codeHash: sha256(code.toU8a(true)), death: era.asMortalEra.death(Number(pin.number)).toString() } };
  return new Journal(path.join(ctx.app, 'intents', flags.intentId + '.json'), intent);
}


async function pingReply(ctx: Context, journal: Journal): Promise<{ payload: Hex; pin: Pin; id: Hex }> {
  const intent = journal.intent;
  check(intent.chain === 'gear' && intent.inclusion && intent.requestId, 'original finalized Gear request unavailable');
  await assertSourcePin(ctx.gear, ctx.witness, intent.inclusion);
  const requestBlock = await ctx.gear.rpc.chain.getBlock(intent.inclusion.hash);
  const requestIndex = requestBlock.block.extrinsics.findIndex(tx => tx.hash.toHex() === intent.hash && tx.toHex() === intent.signedBytes);
  check(requestIndex >= 0, 'original signed Gear request absent from canonical inclusion');
  const requestEvents = (await ctx.gear.query.system.events.at(intent.inclusion.hash)).filter(event => event.phase.isApplyExtrinsic && event.phase.asApplyExtrinsic.toNumber() === requestIndex);
  check(requestEvents.some(({ event }) => event.section === 'system' && event.method === 'ExtrinsicSuccess'), 'original request dispatch failed');
  const requests = requestEvents.filter(({ event }) => event.section === 'gear' && event.method === 'MessageQueued');
  check(requests.length === 1, 'original request identity is ambiguous');
  const request = requests[0].event.data as unknown as { id: { toHex(): Hex }; source: { toHex(): Hex }; destination: { toHex(): Hex } };
  check(request.id.toHex() === intent.requestId && request.source.toHex() === intent.signer && request.destination.toHex() === intent.destination, 'original request identity mismatch');
  if (intent.replyCursor) await assertSourcePin(ctx.gear, ctx.witness, intent.replyCursor);
  let next = BigInt(intent.replyCursor?.number ?? intent.inclusion.number);
  const deadline = ctx.flags.mode === 'verify' ? Date.now() + 60_000 : Number(intent.deadlineAtMs);
  while (true) {
    check(Date.now() < deadline, 'original reply observation deadline exhausted');
    const final = await sourcePin(ctx.gear, ctx.witness);
    for (; next <= BigInt(final.number); next++) {
      check(Date.now() < deadline, 'original reply observation deadline exhausted');
      const hash = (await ctx.gear.rpc.chain.getBlockHash(next.toString())).toHex();
      check((await ctx.witness.rpc.chain.getBlockHash(next.toString())).toHex() === hash, 'reply scan witness disagreement');
      const events = await ctx.gear.query.system.events.at(hash);
      const replies = events.filter(({ event }) => {
        if (event.section !== 'gear' || event.method !== 'UserMessageSent') return false;
        const message = (event as unknown as import('@gear-js/api').UserMessageSent).data.message;
        return message.details.isSome && message.details.unwrap().to.toHex() === intent.requestId;
      });
      check(replies.length <= 1, 'ambiguous original reply');
      if (!replies.length) { intent.replyCursor = { number: next.toString(), hash }; if (ctx.flags.mode !== 'verify') journal.save(); continue; }
      const message = (replies[0].event as unknown as import('@gear-js/api').UserMessageSent).data.message;
      check(message.source.toHex() === intent.destination && message.destination.toHex() === intent.signer && message.details.unwrap().code.isSuccess, 'original reply runtime failure or identity mismatch');
      return { payload: message.payload.toHex(), pin: { number: next.toString(), hash }, id: message.id.toHex() };
    }
    await delay(3000);
  }
}
async function deploymentPreparation(ctx: Context, deadline: string): Promise<Preparation> {
  const filename = path.join(ctx.app, 'preparation.json');
  if (fs.existsSync(filename)) {
    check(ctx.flags.resume, 'original deployment preparation exists; resume only');
    const saved = load<Preparation>(filename);
    check(saved.intentId === ctx.flags.intentId && saved.runId === ctx.run.runId && saved.campaignName === CAMPAIGN, 'different deployment intent');
    for (const [file, hash] of Object.entries(saved.descriptorHashes)) check(sha256(fs.readFileSync(path.join(ctx.root, file))) === hash, 'deployment descriptor changed');
    check(saved.buildManifestDigest === sha256(fs.readFileSync(path.join(ctx.artifacts, 'manifest.json'))), 'deployment artifacts changed');
    return saved;
  }
  check(!ctx.flags.resume, 'deployment preparation missing');
  const accounts = credentials(ctx), pin = await sourcePin(ctx.gear, ctx.witness), final = await ctx.client.getBlock({ blockTag: 'finalized' });
  const code = fs.readFileSync(path.join(ctx.artifacts, 'ping.opt.wasm'));
  const salt = bytesToHex(randomBytes(32)), codeId = generateCodeHash(code), programId = generateProgramId(codeId, salt);
  const nonce = await ctx.client.getTransactionCount({ address: accounts.ethereum.address, blockTag: 'pending' });
  check(nonce === await ctx.client.getTransactionCount({ address: accounts.ethereum.address, blockTag: 'latest' }), 'deployment signer busy');
  const address = getContractAddress({ from: accounts.ethereum.address, nonce: BigInt(nonce) });
  const programCodeIds: Record<string, Hex> = {};
  for (const id of [ctx.stack.checkpoint, ...Object.values(ctx.stack.programs).map(p => p.id)]) programCodeIds[id] = await programCodeId(ctx.gear, pin, id);
  const evmCodeHashes: Record<string, Hex> = {};
  let queueImplementation: Address | undefined;
  for (const [role, id] of Object.entries({ queue: ctx.core.ethereum.queue, client: ctx.core.ethereum.client, verifier: ctx.core.ethereum.verifier })) {
    const hash = keccak256(await ctx.client.getCode({ address: id, blockNumber: final.number! }) ?? '0x');
    check(hash === ctx.core.ethereum.bytecodeHashes[role], 'retained EVM code differs from original deployment'); evmCodeHashes[id] = hash;
    if (role === 'queue') {
      const storage = await ctx.client.getStorageAt({ address: id, slot: IMPLEMENTATION_SLOT, blockNumber: final.number! });
      check(storage && BigInt(storage) !== 0n, 'queue implementation is unavailable');
      const implementation = ('0x' + storage.slice(-40)) as Address;
      queueImplementation = implementation;
      evmCodeHashes[implementation] = keccak256(await ctx.client.getCode({ address: implementation, blockNumber: final.number! }) ?? '0x');
    }
  }
  const rawCode = await ctx.gear.rpc.state.getStorage<Bytes>(':code', pin.hash);
  check(queueImplementation, 'queue implementation unavailable');
  if (ctx.approvedProfile) {
    const approved = ctx.approvedProfile, runtime = ctx.run.runtimeProfile!;
    check(sha256(rawCode.toU8a(true)) === runtime.runtimeCodeSha256.replace(/^0x/, '') &&
      blake2AsHex(rawCode.toU8a(true), 256) === approved.sourceRuntimeCodeHash &&
      keccak256(rawCode.toU8a(true)).slice(2) === runtime.runtimeCodeKeccak256.replace(/^0x/, ''), 'HOLD: normal runtime differs from independently approved labeled hashes');
    for (const [id, expected] of [[approved.historicalProxyId, approved.historicalProxyCodeId],
      [approved.endpoint.programId, approved.endpoint.codeId], [approved.checkpoint.programId, approved.checkpoint.codeId],
      [approved.consumer.programId, approved.consumer.codeId], ...(approved.nativeWrapper ? [[approved.nativeWrapper.programId, approved.nativeWrapper.codeId]] : [])]) {
      check(await programCodeId(ctx.gear, pin, id as Hex) === expected, 'HOLD: normal actor differs from independently approved CodeId');
    }
  }
  const endpoint = { programId: ctx.stack.programs.ethEventsElectra.id, codeId: programCodeIds[ctx.stack.programs.ethEventsElectra.id], idlSha256: sha256(fs.readFileSync(path.join(ctx.artifacts, 'eth_events_electra.idl'))) };
  const prep: Preparation = { schemaVersion: 1, testOnly: true, runId: ctx.run.runId, campaignName: CAMPAIGN, intentId: ctx.flags.intentId, deadlineAtMs: deadline,
    descriptorHashes: Object.fromEntries(['run.json', 'deployment.json', 'token-stack/token-stack.json'].map(file => [file, sha256(fs.readFileSync(path.join(ctx.root, file)))])),
    buildManifestDigest: sha256(fs.readFileSync(path.join(ctx.artifacts, 'manifest.json'))), source: pin, ethereum: { number: final.number!.toString(), hash: final.hash! },
    sourceRuntimeSha256: sha256(rawCode.toU8a(true)), sourceRuntimeCodeHash: blake2AsHex(rawCode.toU8a(true), 256), sourceGenesis: ctx.core.anchor.sourceGenesis, bridgeDomain: ctx.core.anchor.bridgeDomain,
    queue: ctx.core.ethereum.queue, queueImplementation, client: ctx.core.ethereum.client, verifier: ctx.core.ethereum.verifier, proxy: ctx.stack.programs.historicalProxy.id, checkpoint: ctx.stack.checkpoint,
    beacon: HOODI_BEACON, endpoint, checkpointIdlSha256: sha256(fs.readFileSync(path.join(ctx.artifacts, 'checkpoint_light_client.idl'))),
    programCodeIds, evmCodeHashes, caller: { ethereum: accounts.ethereum.address, gear: bytesToHex(accounts.gear.publicKey) },
    ping: { salt, codeId, programId, wasmSha256: sha256(code), config: await actualBridgeConfig(ctx, pin) },
    handler: { address, nonce: String(nonce), artifactSha256: sha256(fs.readFileSync(path.join(ctx.artifacts, 'MessageHandler.json'))), constructor: [ctx.core.ethereum.queue, programId, accounts.ethereum.address] } };
  persist(filename, prep);
  return prep;
}
async function deploy(ctx: Context, deadline: string): Promise<void> {
  check(ctx.direction === 'eth-to-vara', 'deployment belongs to inbound entrypoint');
  if (ctx.deployment) {
    check((ctx.flags.resume || ctx.flags.mode === 'verify') && ctx.deployment.intentId === ctx.flags.intentId, 'application already deployed by another intent');
    await authenticateDeployment(ctx);
    await assertSourcePin(ctx.gear, ctx.witness, ctx.deployment.deploymentIntents.gear.replyPin);
    const binding = ctx.binding; ctx.binding = ctx.deployment.preparationDigest;
    const create = originalIntent(ctx, { mode: 'verify', intentId: ctx.flags.intentId + '-create', resume: false })!;
    const upload = originalIntent(ctx, { mode: 'verify', intentId: ctx.flags.intentId + '-upload', resume: false })!;
    check(create.intent.hash === ctx.deployment.deploymentIntents.ethereum.hash && upload.intent.hash === ctx.deployment.deploymentIntents.gear.hash, 'deployment originals changed');
    const original = await canonicalEthereumReceipt(ctx, create.intent);
    check(original.status === 'success' && original.contractAddress?.toLowerCase() === ctx.deployment.handler.address.toLowerCase(), 'original deployment receipt changed');
    const reply = await pingReply({ ...ctx, flags: { ...ctx.flags, mode: 'verify' } }, upload);
    const init = ctx.gear.createType('(String,Null)', reply.payload);
    check(init.toHex() === reply.payload && (init.toJSON() as unknown[])[0] === 'New' && reply.id === ctx.deployment.deploymentIntents.gear.replyId && reply.pin.hash === ctx.deployment.deploymentIntents.gear.replyPin.hash, 'original Ping initialization reply changed');
    if (ctx.flags.mode !== 'verify') { create.intent.state = 'application-completed'; create.save(); upload.intent.state = 'application-completed'; upload.save(); }
    ctx.binding = binding;
    console.log(stringify({ outcome: 'completed', operation: 'deploy', deploymentDigest: ctx.binding, ping: ctx.deployment.ping.programId, messageHandler: ctx.deployment.handler.address, transactions: ctx.deployment.deploymentIntents })); return;
  }
  const prep = await deploymentPreparation(ctx, deadline);
  check(Date.now() < Number(prep.deadlineAtMs), 'original deployment deadline expired');
  ctx.binding = sha256(privateBytes(path.join(ctx.app, 'preparation.json')));
  const createFlags: Flags = { mode: 'deploy', intentId: ctx.flags.intentId + '-create', resume: ctx.flags.resume && fs.existsSync(path.join(ctx.app, 'intents', ctx.flags.intentId + '-create.json')) };
  check(NAME.test(createFlags.intentId), 'deployment intent ID exceeds sub-intent bound');
  const data = encodeDeployData({ abi: ctx.handlerAbi, bytecode: ctx.handlerBytecode, args: prep.handler.constructor });
  let create = originalIntent(ctx, createFlags);
  if (!create) create = await signEthereum(ctx, data, null, 'MessageHandler.constructor', prep.deadlineAtMs, 0n, createFlags);
  check(create.intent.nonce === prep.handler.nonce && create.intent.calldata === data && create.intent.destination === null && create.intent.value === '0', 'original CREATE preparation changed');
  await broadcastEthereum(create, ctx.client);
  const receipt = await canonicalEthereumReceipt(ctx, create.intent);
  check(receipt.status === 'success' && receipt.contractAddress?.toLowerCase() === prep.handler.address.toLowerCase(), 'original CREATE failed or address mismatch');
  const uploadFlags: Flags = { mode: 'deploy', intentId: ctx.flags.intentId + '-upload', resume: ctx.flags.resume && fs.existsSync(path.join(ctx.app, 'intents', ctx.flags.intentId + '-upload.json')) };
  let upload = originalIntent(ctx, uploadFlags);
  if (!upload) {
    const program = new PingClient(ctx.gear);
    const code = fs.readFileSync(path.join(ctx.artifacts, 'ping.opt.wasm'));
    check(sha256(code) === prep.ping.wasmSha256, 'original Ping WASM changed');
    const result = program.newCtorFromCode(code, prep.ping.salt, prep.proxy, prep.handler.address, prep.caller.ethereum, prep.handler.address, prep.ping.config, ctx.gear.blockGasLimit.toBigInt() / 100n * 95n);
    const initPayload = result.extrinsic.args[2].toHex();
    check(result.programId === prep.ping.programId && result.codeId === prep.ping.codeId, 'predicted Ping upload identity mismatch');
    upload = await signGear(ctx, result.extrinsic, prep.ping.programId, 'New', initPayload, 0n, prep.deadlineAtMs, uploadFlags);
  }
  await broadcastGear(upload, ctx.gear, ctx.witness);
  const reply = await pingReply(ctx, upload);
  const init = ctx.gear.createType('(String,Null)', reply.payload);
  check(init.toHex() === reply.payload && (init.toJSON() as unknown[])[0] === 'New', 'original Ping initialization reply mismatch');
  check(await programCodeId(ctx.gear, reply.pin, prep.ping.programId) === prep.ping.codeId, 'finalized deployed Ping CodeId mismatch');
  const runtime = await ctx.client.getCode({ address: prep.handler.address, blockNumber: BigInt(create.intent.inclusion!.number) });
  check(runtime && runtime !== '0x', 'finalized MessageHandler code absent');
  const deployment: Deployment = { ...prep, preparationDigest: ctx.binding, handlerRuntimeHash: keccak256(runtime), deploymentIntents: {
    ethereum: { hash: create.intent.hash, pin: create.intent.inclusion! }, gear: { hash: upload.intent.hash, pin: upload.intent.inclusion!, requestId: upload.intent.requestId!, replyId: reply.id, replyPin: reply.pin } } };
  check(Date.now() < Number(prep.deadlineAtMs), 'original deployment deadline exhausted');
  persist(path.join(ctx.app, 'deployment.json'), deployment);
  ctx.deployment = deployment; ctx.binding = sha256(privateBytes(path.join(ctx.app, 'deployment.json')));
  create.intent.state = 'application-completed'; create.save(); upload.intent.state = 'application-completed'; upload.save();
  console.log(stringify({ outcome: 'completed', operation: 'deploy', deploymentDigest: ctx.binding, ping: prep.ping.programId, messageHandler: prep.handler.address, transactions: deployment.deploymentIntents }));
}


type MessageEvidence = { outcome: 'completed'; applicationId: Hex; payload: Hex; blockNumber?: string; nonce?: string; hash?: Hex; queueId?: string };
type RelayEvidence = {
  sourceIntent: string; applicationId: Hex; payload: Hex;
  before?: { pin: Pin; received: unknown; payload: Hex | null; processed?: boolean };
  inbound?: { slot: string; transactionIndex: string; receiptRlp: Hex; endpoint: Hex; checkpoint: Hex; expectedProofError?: 'TrieDbFailure' | 'InvalidReceiptProof' };
  outbound?: { blockNumber: string; message: { nonce: string; source: Hex; destination: Hex; payload: Hex }; proof: { root: Hex; proof: Hex[]; numLeaves: string; leafIndex: string } };
};
async function sourceMessage(ctx: Context, id: string): Promise<Intent & { result: MessageEvidence }> {
  const journal = originalIntent(ctx, { mode: 'verify', intentId: id, resume: false })!, source = journal.intent;
  check(source.schemaVersion === 1 && source.runId === ctx.run.runId && source.deploymentDigest === ctx.binding && source.direction === ctx.direction
    && source.operation === 'send' && source.state === 'application-completed' && source.result && source.applicationId && source.payload !== undefined, 'original finalized source send unavailable');
  const result = source.result as MessageEvidence;
  check(result.applicationId === source.applicationId && result.payload === source.payload, 'source result changed');
  check(source.inclusion, 'source request finality missing');
  if (source.chain === 'ethereum') check((await ctx.client.getBlock({ blockNumber: BigInt(source.inclusion.number) })).hash === source.inclusion.hash, 'original EVM source changed');
  else await assertSourcePin(ctx.gear, ctx.witness, source.inclusion);
  check(canonical(await completeSend({ ...ctx, flags: { ...ctx.flags, mode: 'verify' } }, journal)) === canonical(result), 'original source application evidence changed');
  return source as Intent & { result: MessageEvidence };
}
function eventLogs(receipt: Awaited<ReturnType<PublicClient['getTransactionReceipt']>>, address: Address, abi: Abi, name: string) {
  return receipt.logs.filter(log => log.address.toLowerCase() === address.toLowerCase()).flatMap(log => {
    try { const decoded = decodeEventLog({ abi, data: log.data, topics: log.topics, strict: true }); return decoded.eventName === name ? [decoded.args as unknown as Record<string, unknown>] : []; }
    catch { return []; }
  });
}
async function completeSend(ctx: Context, journal: Journal): Promise<MessageEvidence> {
  const intent = journal.intent, deployment = ctx.deployment!;
  check(intent.applicationId && intent.payload !== undefined && intent.inclusion, 'source intent missing original message fields');
  if (intent.chain === 'ethereum') {
    const receipt = await canonicalEthereumReceipt(ctx, intent);
    check(receipt.status === 'success' && receipt.blockHash === intent.inclusion.hash, 'source request not finalized successfully');

    const events = eventLogs(receipt, deployment.handler.address, ctx.handlerAbi, 'MessageRequested');
    check(events.length === 1 && events[0].applicationId === intent.applicationId && events[0].payload === intent.payload
      && String(events[0].sender).toLowerCase() === deployment.caller.ethereum.toLowerCase() && events[0].destination === deployment.ping.programId, 'authenticated MessageRequested mismatch');
    return { outcome: 'completed', applicationId: intent.applicationId, payload: intent.payload };
  }
  const ping = new PingClient(ctx.gear, deployment.ping.programId), reply = await pingReply(ctx, journal);
  const decoded = ping.decodeReply('SendMessage', reply.payload);
  check('ok' in decoded, 'Ping SendMessage rejected or unresolved');
  const queued = decoded.ok;
  check(queued && queued.block_number !== undefined && queued.nonce !== undefined && queued.queue_id !== undefined, 'invalid queued delivery reply');
  const blockNumber = BigInt(queued.block_number), nonce = BigInt(queued.nonce);
  const hash = (await ctx.gear.rpc.chain.getBlockHash(blockNumber.toString())).toHex();
  await assertSourcePin(ctx.gear, ctx.witness, { number: blockNumber.toString(), hash });
  const events = await ctx.gear.query.system.events.at(hash);
  const packed = (intent.applicationId + intent.payload.slice(2)) as Hex;
  const messages = events.filter(({ event }) => event.section === 'gearEthBridge' && event.method === 'MessageQueued').filter(({ event }) => {
    const data = event.data as unknown as { message: { nonce: { toString(): string }; source: { toHex(): Hex }; destination: { toHex(): Hex }; payload: { toHex(): Hex } } };
    return BigInt(data.message.nonce.toString()) === nonce && data.message.source.toHex() === deployment.ping.programId
      && data.message.destination.toHex().toLowerCase() === deployment.handler.address.toLowerCase() && data.message.payload.toHex() === packed && event.data[1].toHex() === queued.hash;
  });
  check(messages.length === 1, 'original builtin queue message mismatch');
  const outbound = await ping.ping.outbound(intent.applicationId).atBlock(reply.pin.hash).call();
  check(canonical(outbound) === canonical({ queued }), 'same-pin outbound application readback mismatch');
  intent.replyId = reply.id; intent.replyPin = reply.pin;
  return { outcome: 'completed', applicationId: intent.applicationId, payload: intent.payload, blockNumber: blockNumber.toString(), nonce: nonce.toString(), hash: queued.hash, queueId: BigInt(queued.queue_id).toString() };
}
async function newSend(ctx: Context, deadline: string): Promise<Journal> {
  const deployment = ctx.deployment!, flags = ctx.flags;
  if (ctx.direction === 'eth-to-vara') {
    const calldata = encodeFunctionData({ abi: ctx.handlerAbi, functionName: 'sendMessage', args: [flags.applicationId!, flags.payloadHex!] });
    return signEthereum(ctx, calldata, deployment.handler.address, 'MessageHandler.sendMessage', deadline);
  }
  const ping = new PingClient(ctx.gear, deployment.ping.programId);
  const value = BigInt(deployment.ping.config.fee_bridge);
  const transaction = ping.ping.sendMessage(flags.applicationId!, flags.payloadHex!).withValue(value).withGas(ctx.gear.blockGasLimit.toBigInt() / 100n * 95n);
  return signGear(ctx, transaction.extrinsic, deployment.ping.programId, 'Ping.SendMessage', transaction.extrinsic.args[1].toHex(), value, deadline);
}
async function newRelay(ctx: Context, deadline: string): Promise<Journal> {
  const source = await sourceMessage(ctx, ctx.flags.sourceIntent!), deployment = ctx.deployment!;
  const evidence: RelayEvidence = { sourceIntent: source.intentId, applicationId: source.result.applicationId, payload: source.result.payload };
  let journal: Journal;
  if (ctx.direction === 'eth-to-vara') {
    const { prepareEthToVaraRelay } = await import('../src/eth-to-vara/relayer.js');
    const { HistoricalProxyClient, EthEventsClient, encodeEthToVaraEvent } = await import('../src/vara/index.js');
    const profilePin = await sourcePin(ctx.gear, ctx.witness);
    const inboundProfile = { ethereumChainId: 560048n, ethereumGenesisHash: ctx.run.network.genesisHash, beaconGenesisValidatorsRoot: deployment.beacon.genesisValidatorsRoot,
      beaconGenesisTime: 1742213400n,
      sourceGenesisHash: deployment.sourceGenesis, sourceBlockHash: profilePin.hash, sourceRuntimeCodeHash: deployment.sourceRuntimeCodeHash,
      historicalProxyId: deployment.proxy, historicalProxyCodeId: deployment.programCodeIds[deployment.proxy],
      historicalProxyIdlSha256: sha256(fs.readFileSync(path.join(ctx.artifacts, 'historical_proxy.idl'))),
      endpoint: { ...deployment.endpoint, framing: 'electra' as const }, checkpoint: { programId: deployment.checkpoint, codeId: deployment.programCodeIds[deployment.checkpoint], idlSha256: deployment.checkpointIdlSha256, network: 'Hoodi' as const },
      consumer: { programId: deployment.ping.programId, codeId: deployment.ping.codeId, idlSha256: sha256(ctx.pingIdl), service: 'Ping', method: 'SubmitReceipt' },
      forks: deployment.beacon.forks.map(f => ({ ...f, epoch: BigInt(f.epoch) })) };
    const approvedInboundProfile = ctx.approvedProfile ? immutableInboundProfile({ ...ctx.approvedProfile,
      consumer: inboundProfile.consumer }) : inboundProfile;
    const prepared = await prepareEthToVaraRelay({ transactionHash: source.hash, beaconRpcUrl: ctx.run.network.beaconHttp, ethereumPublicClient: ctx.client, inboundProfile: approvedInboundProfile, deadline: Number(deadline),
      gearApi: ctx.gear, historicalProxyId: deployment.proxy, clientId: deployment.ping.programId, clientServiceName: 'Ping', clientMethodName: 'SubmitReceipt', wait: true });
    check(Date.now() < Number(deadline), 'proof preparation exhausted original deadline');
    const slot = BigInt(prepared.proof.proofBlock.block.slot), index = BigInt(prepared.proof.transactionIndex), pin = await sourcePin(ctx.gear, ctx.witness);
    const proxy = new HistoricalProxyClient(ctx.gear, deployment.proxy);
    check(slot <= BigInt(Number.MAX_SAFE_INTEGER), 'receipt slot outside endpoint query range');
    const endpointResult = await proxy.historicalProxy.endpointFor(Number(slot)).atBlock(pin.hash).call();
    check('ok' in endpointResult, 'historical proof endpoint unavailable');
    const endpoint = endpointResult.ok;
    check(deployment.programCodeIds[endpoint] && await programCodeId(ctx.gear, pin, endpoint) === deployment.programCodeIds[endpoint], 'unqualified receipt endpoint');
    const ethEvents = new EthEventsClient(ctx.gear, endpoint);
    const checkpoint = await ethEvents.ethereumEventClient.checkpointLightClientAddress().atBlock(pin.hash).call();
    check(checkpoint === deployment.checkpoint, 'receipt endpoint checkpoint mismatch');
    evidence.inbound = { slot: slot.toString(), transactionIndex: index.toString(), receiptRlp: bytesToHex(prepared.proof.receiptRlp), endpoint, checkpoint };
    const app = new PingClient(ctx.gear, deployment.ping.programId);
    evidence.before = { pin, received: await app.ping.received(slot, index).atBlock(pin.hash).call(), payload: await app.ping.payloadOf(evidence.applicationId).atBlock(pin.hash).call() };
    if (ctx.flags.case === 'duplicate') check(evidence.before.received !== null && evidence.before.payload === evidence.payload, 'duplicate receipt probe requires original completed delivery');
    let transaction: typeof prepared.transaction | ReturnType<PingClient['ping']['submitReceipt']> = prepared.transaction;
    let destination = deployment.proxy, route = 'HistoricalProxy.Redirect';
    if (ctx.flags.case === 'tampered-proof') {
      const node = prepared.proof.proof[prepared.proof.proof.length - 1];
      check(node && node.length > 1, 'receipt proof has no mutable MPT node');
      node[node.length - 1] ^= 1;
      evidence.inbound.expectedProofError = prepared.proof.proof.length === 1 ? 'TrieDbFailure' : 'InvalidReceiptProof';
      transaction = proxy.historicalProxy.redirect(slot, encodeEthToVaraEvent(prepared.proof), deployment.ping.programId, ctx.gear.createType('(String,String)', ['Ping', 'SubmitReceipt']).toHex());
    } else if (ctx.flags.case === 'direct-client') {
      const ping = new PingClient(ctx.gear, deployment.ping.programId);
      transaction = ping.ping.submitReceipt(slot, index, evidence.inbound.receiptRlp);
      destination = deployment.ping.programId; route = 'Ping.SubmitReceipt';
    }
    transaction.withGas(ctx.gear.blockGasLimit.toBigInt() / 100n * 95n).withValue(0n);
    const payload = transaction.extrinsic.args[1].toHex();
    journal = await signGear(ctx, transaction.extrinsic, destination, route, payload, 0n, deadline, ctx.flags, evidence);
  } else {
    const { prepareVaraToEthRelay } = await import('../src/vara-to-eth/relayer.js');
    const { getProcessMessageArgs } = await import('../src/ethereum/message-queue.js');
    check(source.result.nonce !== undefined && source.result.blockNumber !== undefined, 'original builtin coordinates absent');
    const prepared = await prepareVaraToEthRelay({ nonce: BigInt(source.result.nonce), blockNumber: BigInt(source.result.blockNumber),
      ethereumPublicClient: ctx.client, gearApi: ctx.gear, messageQueueAddress: deployment.queue, wait: true, deadline: Number(deadline) });
    check(Date.now() < Number(deadline), 'root proof preparation exhausted original deadline');
    const packed = (source.result.applicationId + source.result.payload.slice(2)) as Hex;
    check(bytesToHex(prepared.message.source) === deployment.ping.programId && bytesToHex(prepared.message.destination).toLowerCase() === deployment.handler.address.toLowerCase() && bytesToHex(prepared.message.payload) === packed, 'proof substituted original application message');
    const beforeBlock = await ctx.client.getBlock({ blockTag: 'finalized' });
    const appRead = { address: deployment.handler.address, abi: ctx.handlerAbi, args: [evidence.applicationId], blockNumber: beforeBlock.number! };
    evidence.before = { pin: { number: beforeBlock.number!.toString(), hash: beforeBlock.hash! },
      received: await ctx.client.readContract({ ...appRead, functionName: 'received' }), payload: await ctx.client.readContract({ ...appRead, functionName: 'payloadOf' }) as Hex,
      processed: await ctx.client.readContract({ address: deployment.queue, abi: load<{ abi: Abi }>(path.join(ctx.artifacts, 'MessageQueue.json')).abi, functionName: 'isProcessed', args: [prepared.message.nonce], blockNumber: beforeBlock.number! }) as boolean };
    if (ctx.flags.case === 'duplicate') check(evidence.before.processed === true && evidence.before.received === true && evidence.before.payload === evidence.payload, 'duplicate queue probe requires original completed delivery');
    if (ctx.flags.case === 'tampered-proof') {
      const processed = await ctx.client.readContract({ address: deployment.queue, abi: load<{ abi: Abi }>(path.join(ctx.artifacts, 'MessageQueue.json')).abi, functionName: 'isProcessed', args: [prepared.message.nonce], blockTag: 'finalized' });
      check(processed === false, 'tampered-proof probe must precede original delivery');
      if (prepared.proof.proof.length) prepared.proof.proof[0] = ('0x' + (BigInt(prepared.proof.proof[0]) ^ 1n).toString(16).padStart(64, '0')) as Hex;
      else prepared.message.payload[prepared.message.payload.length - 1] ^= 1;
    }
    evidence.outbound = { blockNumber: prepared.blockNumber.toString(), message: { nonce: prepared.message.nonce.toString(), source: bytesToHex(prepared.message.source), destination: bytesToHex(prepared.message.destination), payload: bytesToHex(prepared.message.payload) },
      proof: { root: prepared.proof.root, proof: prepared.proof.proof, numLeaves: prepared.proof.numLeaves.toString(), leafIndex: prepared.proof.leafIndex.toString() } };
    const direct = ctx.flags.case === 'direct-receiver';
    const abi = direct ? ctx.handlerAbi : load<{ abi: Abi }>(path.join(ctx.artifacts, 'MessageQueue.json')).abi;
    const calldata = direct ? encodeFunctionData({ abi, functionName: 'handleMessage', args: [deployment.ping.programId, packed] })
      : encodeFunctionData({ abi, functionName: 'processMessage', args: getProcessMessageArgs(prepared.blockNumber, prepared.message, prepared.proof) });
    journal = await signEthereum(ctx, calldata, direct ? deployment.handler.address : deployment.queue, direct ? 'MessageHandler.handleMessage' : 'MessageQueue.processMessage', deadline, 0n, ctx.flags, evidence);
  }
  return journal;
}
async function completeRelay(ctx: Context, journal: Journal): Promise<unknown> {
  const intent = journal.intent, evidence = intent.result as RelayEvidence, deployment = ctx.deployment!;
  check(evidence && evidence.applicationId === intent.applicationId && evidence.payload === intent.payload && evidence.sourceIntent === intent.arguments.sourceIntent, 'original relay preparation missing');
  if (ctx.direction === 'eth-to-vara') {
    check(evidence.inbound && intent.requestId && intent.inclusion, 'original receipt request missing');
    const ping = new PingClient(ctx.gear, deployment.ping.programId);
    let clientReply: Hex | undefined, replyPin: Pin, replyId: Hex, proxyError: unknown;
    if (intent.arguments.case === 'direct-client') {
      const reply = await pingReply(ctx, journal); clientReply = reply.payload; replyPin = reply.pin; replyId = reply.id;
    } else {
      const { validateFinalizedEthToVaraReply } = await import('../src/eth-to-vara/relayer.js');
      const reply = await validateFinalizedEthToVaraReply({ gearApi: ctx.gear, historicalProxyId: deployment.proxy, sender: intent.signer as Hex,
        msgId: intent.requestId, blockHash: intent.inclusion.hash, txHash: intent.hash, requestPayload: intent.calldata,
        receiptRlp: hexToBytes(evidence.inbound.receiptRlp), deadline: ctx.flags.mode === 'verify' ? Date.now() + 60_000 : Number(intent.deadlineAtMs) });
      clientReply = reply.clientReply; proxyError = reply.error;
      replyPin = { number: reply.replyBlockNumber.toString(), hash: reply.replyBlockHash as Hex }; replyId = reply.replyMessageId;
    }
    await assertSourcePin(ctx.gear, ctx.witness, replyPin);
    const decoded = clientReply ? ping.decodeReply('SubmitReceipt', clientReply) : undefined;
    if (intent.operation === 'probe') {
      const expected = intent.arguments.case === 'direct-client' ? 'NotHistoricalProxy' : intent.arguments.case === 'duplicate' ? 'AlreadyProcessed' : evidence.inbound.expectedProofError;
      if (intent.arguments.case === 'tampered-proof') check(expected && canonical(proxyError) === canonical({ EthereumEventClient: expected }) && !clientReply, 'proof probe rejected at wrong layer');
      else check(decoded && 'err' in decoded && decoded.err === expected, 'application probe rejected at wrong layer');
      const stored = await ping.ping.payloadOf(intent.applicationId!).atBlock(replyPin.hash).call();
      check(evidence.before, 'original probe pre-state absent');
      await assertSourcePin(ctx.gear, ctx.witness, evidence.before.pin);
      const received = await ping.ping.received(BigInt(evidence.inbound.slot), BigInt(evidence.inbound.transactionIndex)).atBlock(replyPin.hash).call();
      check(stored === evidence.before.payload && canonical(received) === canonical(evidence.before.received), 'rejection changed application state');
      intent.replyId = replyId; intent.replyPin = replyPin;
      return { outcome: 'rejected', expectedError: expected, proxyError, clientReply, decoded, ...evidence, replyId, replyPin };
    }
    check(!proxyError && decoded && 'ok' in decoded, 'consumer has not returned explicit finalized Ok');
    const delivery = decoded.ok;
    check(delivery.application_id === intent.applicationId && delivery.payload === intent.payload, 'inbound exact payload mismatch');
    const received = await ping.ping.received(BigInt(evidence.inbound.slot), BigInt(evidence.inbound.transactionIndex)).atBlock(replyPin.hash).call();
    const stored = await ping.ping.payloadOf(intent.applicationId!).atBlock(replyPin.hash).call();
    check(canonical(received) === canonical(delivery) && canonical(stored) === canonical(delivery.payload), 'same-pin inbound application readback mismatch');
    intent.replyId = replyId; intent.replyPin = replyPin;
    return { outcome: 'completed', clientReply, decoded, ...evidence, replyId, replyPin };
  }
  check(evidence.outbound && intent.inclusion, 'outbound original proof coordinates absent');
  const prepared = evidence.outbound;
  const message = { nonce: BigInt(prepared.message.nonce), source: hexToBytes(prepared.message.source), destination: hexToBytes(prepared.message.destination), payload: hexToBytes(prepared.message.payload) };
  const proof = { ...prepared.proof, numLeaves: BigInt(prepared.proof.numLeaves), leafIndex: BigInt(prepared.proof.leafIndex) };
  if (intent.operation === 'probe') {
    const receipt = await canonicalEthereumReceipt(ctx, intent);
    check(receipt.status === 'reverted' && receipt.blockHash === intent.inclusion.hash, 'negative probe did not finalize reverted');
    const direct = intent.arguments.case === 'direct-receiver';
    const abi = direct ? ctx.handlerAbi : load<{ abi: Abi }>(path.join(ctx.artifacts, 'MessageQueue.json')).abi;
    const { decodeFunctionData, BaseError, ContractFunctionRevertedError } = await import('viem');
    const decoded = decodeFunctionData({ abi, data: intent.calldata });
    const expected = direct ? 'NotQueue' : intent.arguments.case === 'duplicate' ? 'MessageAlreadyProcessed' : 'InvalidMerkleProof';
    const errors: string[] = [];
    for (const blockNumber of [receipt.blockNumber - 1n, receipt.blockNumber]) {
      try { await ctx.client.simulateContract({ address: intent.destination as Address, abi, functionName: decoded.functionName!, args: decoded.args, account: intent.signer as Address, value: BigInt(intent.value), blockNumber }); }
      catch (error) {
        const reverted = error instanceof BaseError ? error.walk(e => e instanceof ContractFunctionRevertedError) : undefined;
        if (reverted instanceof ContractFunctionRevertedError && reverted.data) errors.push(reverted.data.errorName);
        else throw error;
      }
    }
    check(errors.length === 2 && errors.every(error => error === expected), 'original finalized probe has no matching pre/post-state intended rejection');
    const queueAbi = load<{ abi: Abi }>(path.join(ctx.artifacts, 'MessageQueue.json')).abi;
    check(evidence.before && (await ctx.client.getBlock({ blockNumber: BigInt(evidence.before.pin.number) })).hash === evidence.before.pin.hash, 'original probe pre-state changed');
    const appRead = { address: deployment.handler.address, abi: ctx.handlerAbi, args: [intent.applicationId], blockNumber: receipt.blockNumber };
    check(await ctx.client.readContract({ ...appRead, functionName: 'received' }) === evidence.before.received
      && await ctx.client.readContract({ ...appRead, functionName: 'payloadOf' }) === evidence.before.payload
      && await ctx.client.readContract({ address: deployment.queue, abi: queueAbi, functionName: 'isProcessed', args: [message.nonce], blockNumber: receipt.blockNumber }) === evidence.before.processed, 'rejection changed app or queue replay state');
    if (intent.arguments.case === 'tampered-proof') check(await ctx.client.readContract({ address: deployment.queue, abi: queueAbi, functionName: 'isProcessed', args: [message.nonce], blockNumber: receipt.blockNumber }) === false, 'tampered proof consumed nonce');
    return { outcome: 'rejected', expectedError: expected, rejectionEvidence: 'canonical reverted transaction plus exact call at bracketing historical states', ...evidence, inclusion: intent.inclusion };
  }
  const { validateFinalizedMessageReceipt } = await import('../src/ethereum/message-queue.js');
  const result = await validateFinalizedMessageReceipt({ ethereumPublicClient: ctx.client, messageQueueAddress: deployment.queue, transactionHash: intent.hash,
    blockNumber: BigInt(prepared.blockNumber), message, proof, sender: intent.signer as Hex, expectedEffect: { kind: 'application' }, deadline: ctx.flags.mode === 'verify' ? Date.now() + 60_000 : Number(intent.deadlineAtMs) });
  const receipt = await canonicalEthereumReceipt(ctx, intent);
  const events = eventLogs(receipt, deployment.handler.address, ctx.handlerAbi, 'MessageHandled');
  check(events.length === 1 && events[0].source === deployment.ping.programId && events[0].applicationId === intent.applicationId && events[0].payload === intent.payload, 'configured app MessageHandled mismatch');
  const readback = { address: deployment.handler.address, abi: ctx.handlerAbi, args: [intent.applicationId], blockNumber: result.receiptBlockNumber };
  check(await ctx.client.readContract({ ...readback, functionName: 'received' }) === true && await ctx.client.readContract({ ...readback, functionName: 'payloadOf' }) === intent.payload, 'same-pin outbound application readback mismatch');
  return { outcome: 'completed', ...evidence, queueReceipt: result, inclusion: intent.inclusion };
}
function publishResult(ctx: Context, journal: Journal, result: unknown): void {
  const intent = journal.intent;
  const record = { schemaVersion: 1, testOnly: true, runId: ctx.run.runId, campaignName: CAMPAIGN, direction: ctx.direction, intentId: intent.intentId,
    applicationId: intent.applicationId, payload: intent.payload, payloadDigest: intent.payload ? sha256(hexToBytes(intent.payload)) : undefined,
    deploymentDigest: ctx.binding, originalTransactionHash: intent.hash, originalRequestId: intent.requestId, originalInclusion: intent.inclusion,
    originalReplyId: intent.replyId, originalReplyPin: intent.replyPin, result };
  const filename = path.join(ctx.app, 'results', intent.intentId + '.json');
  if (ctx.flags.mode !== 'verify') {
    if (fs.existsSync(filename)) {
      const previous = load<{ originalTransactionHash: Hex; deploymentDigest: string; result: { outcome?: string } }>(filename);
      if (canonical(previous) !== canonical(record)) {
        check(previous.originalTransactionHash === intent.hash && previous.deploymentDigest === ctx.binding && previous.result.outcome === 'HOLD', 'original public result conflicts');
        persist(filename, record, sha256(privateBytes(filename)));
      }
    } else persist(filename, record);
  }
  console.log(stringify(record));
}
export async function runApp(direction: Direction): Promise<void> {
  const flags = parseFlags(process.argv.slice(2));
  if (flags.mode === 'probe') check((direction === 'eth-to-vara' ? ['tampered-proof', 'direct-client', 'duplicate'] : ['tampered-proof', 'direct-receiver', 'duplicate']).includes(flags.case!), 'unknown directional rejection case');
  const deadline = String(Date.now() + MAX_OPERATION_MS);
  await cryptoWaitReady();
  const ctx = await connect(direction, flags);
  let journal: Journal | undefined;
  try {
    if (flags.mode !== 'verify') for (const name of fs.readdirSync(path.join(ctx.app, 'intents'))) {
      const saved = JSON.parse(privateBytes(path.join(ctx.app, 'intents', name)).toString()) as Intent;
      check(saved.schemaVersion === 1 && saved.runId === ctx.run.runId && NAME.test(saved.intentId) && name === saved.intentId + '.json', 'unknown original app journal entry');
      const resuming = flags.resume && (saved.intentId === flags.intentId || (flags.mode === 'deploy' && [flags.intentId + '-create', flags.intentId + '-upload'].includes(saved.intentId)));
      check(['application-completed', 'rejected'].includes(saved.state) || resuming, 'unsettled original app intent: ' + saved.intentId);
    }
    if (flags.mode === 'deploy' || (flags.mode === 'verify' && ctx.deployment?.intentId === flags.intentId)) { await deploy(ctx, deadline); return; }
    await authenticateDeployment(ctx);
    journal = originalIntent(ctx);
    if (!journal) journal = flags.mode === 'send' ? await newSend(ctx, deadline) : await newRelay(ctx, deadline);
    if (flags.mode !== 'verify') {
      withinDeadline(journal.intent);
      if (journal.intent.chain === 'ethereum') await broadcastEthereum(journal, ctx.client);
      else await broadcastGear(journal, ctx.gear, ctx.witness);
    } else check(journal.intent.state === 'application-completed' || journal.intent.state === 'rejected', 'original operation not completed; verify cannot submit');
    const result = journal.intent.operation === 'send' ? await completeSend(ctx, journal) : await completeRelay(ctx, journal);
    if (flags.mode !== 'verify') {
      withinDeadline(journal.intent);
      journal.intent.state = journal.intent.operation === 'probe' ? 'rejected' : 'application-completed';
      if (journal.intent.operation === 'send') journal.intent.result = result;
      journal.save();
    }
    publishResult(ctx, journal, result);
  } catch (error) {
    if (journal && flags.mode !== 'verify' && !['application-completed', 'rejected'].includes(journal.intent.state)) {
      journal.intent.state = 'HOLD'; journal.save();
      publishResult(ctx, journal, { outcome: 'HOLD', reason: error instanceof Error && error.message.startsWith('HOLD:') ? error.message : 'Original outcome unresolved; retain exact signed intent' });
    }
    throw error;
  } finally { await ctx.gear.disconnect(); await ctx.witness.disconnect(); }
}
