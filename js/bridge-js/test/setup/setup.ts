import { createPublicClient, webSocket, PublicClient } from 'viem';
import { execFileSync } from 'child_process';
import * as path from 'path';
import * as fs from 'fs';
import dotenv from 'dotenv';
import { GearApi, HexString } from '@gear-js/api';
import { blake2AsHex } from '@polkadot/util-crypto';
import { QueryBuilder } from 'sails-js';
import { CheckpointClient, EthEventsClient, HistoricalProxyClient, StateData } from '../../src/vara/index.js';
import { createBeaconClient } from '../../src/ethereum/index.js';
import { immutableInboundProfile, InboundProofProfile } from '../../src/eth-to-vara/proof-composer.js';
import type { Bytes } from '@polkadot/types';

dotenv.config();
const PATH_TO_BIN = path.resolve(process.env.CARGO_TARGET_DIR ?? '../../target', 'release/js-test');

export function readApprovedFixtureProfile(filename: string, historicalProxyId: string): InboundProofProfile {
  const supplied = JSON.parse(fs.readFileSync(filename, 'utf8')) as Omit<InboundProofProfile, 'ethereumChainId' | 'beaconGenesisTime' | 'forks'> & {
    ethereumChainId: string; beaconGenesisTime: string; forks: { name: string; epoch: string; version: HexString }[] };
  const approved = immutableInboundProfile({ ...supplied, ethereumChainId: BigInt(supplied.ethereumChainId), beaconGenesisTime: BigInt(supplied.beaconGenesisTime),
    forks: supplied.forks.map(fork => ({ ...fork, epoch: BigInt(fork.epoch) })) });
  if (historicalProxyId !== approved.historicalProxyId || approved.ethereumChainId !== 560048n || approved.checkpoint.network !== 'Hoodi' ||
      approved.ethereumGenesisHash !== '0xbbe312868b376a3001692a646dd2d7d1e4406380dfd86b98aa8a34d1557c971b' ||
      approved.beaconGenesisValidatorsRoot !== '0x212f13fc4df078b6cb7db228f1c8307566dcecf900867401a92023d7ba99cb5f') {
    throw new Error('HOLD: independently approved owned Hoodi fixture/profile is required');
  }
  return approved;
}
const getFixture = async (approved: InboundProofProfile) => {
  const gearApi = await GearApi.create({ providerAddress: process.env.VARA_WS_RPC });
  let publicClient: PublicClient | undefined;
  try {
    if (gearApi.genesisHash.toHex() !== approved.sourceGenesisHash) throw new Error('HOLD: fixture source genesis differs from independent approval');
    const pin = (await gearApi.blocks.getFinalizedHead()).toHex(), tip = await gearApi.rpc.chain.getHeader(pin);
    const anchor = await gearApi.rpc.chain.getHeader(approved.sourceBlockHash);
    if (anchor.number.toBigInt() > tip.number.toBigInt() || (await gearApi.blocks.getBlockHash(anchor.number.toBigInt())).toHex() !== approved.sourceBlockHash) {
      throw new Error('HOLD: approved fixture source pin is not canonical finalized history');
    }
    for (const at of [approved.sourceBlockHash, pin]) {
      const code = await gearApi.rpc.state.getStorage<Bytes>(':code', at);
      if (code.isEmpty || blake2AsHex(code.toU8a(true), 256) !== approved.sourceRuntimeCodeHash) throw new Error('HOLD: approved fixture runtime differs');
    }
    for (const [id, expected] of [[approved.historicalProxyId, approved.historicalProxyCodeId], [approved.endpoint.programId, approved.endpoint.codeId],
      [approved.checkpoint.programId, approved.checkpoint.codeId], [approved.consumer.programId, approved.consumer.codeId],
      ...(approved.nativeWrapper ? [[approved.nativeWrapper.programId, approved.nativeWrapper.codeId]] : [])]) {
      const program = await gearApi.programStorage.getProgram(id as HexString, pin);
      if (!program.state.isInitialized || !program.codeId.eq(expected)) throw new Error('HOLD: fixture actor CodeId differs from approval');
    }
    const historicalProxy = new HistoricalProxyClient(gearApi, approved.historicalProxyId);
    const ethEvents = new EthEventsClient(gearApi, approved.endpoint.programId);
    if (await ethEvents.ethereumEventClient.checkpointLightClientAddress().atBlock(pin).call() !== approved.checkpoint.programId) throw new Error('HOLD: fixture checkpoint differs from approval');
    const checkpoint = new CheckpointClient(gearApi, approved.checkpoint.programId);
    if (await checkpoint.serviceState.network().atBlock(pin).call() !== approved.checkpoint.network) throw new Error('HOLD: checkpoint immutable network differs from approval');
    const state = await new QueryBuilder<StateData>(gearApi, checkpoint.registry, approved.checkpoint.programId,
      'ServiceState', 'Get', ['Reverse', 0, 1], '(Order, u32, u32)', 'StateData').atBlock(pin).call();
    if (state.replay_back || state.checkpoints.length !== 1) throw new Error('Fixture checkpoint has no applied terminal state');
    const beacon = await createBeaconClient(process.env.BEACON_RPC_URL!);
    const spec = await beacon.getSpec();
    if (beacon.genesisBlock.genesis_validators_root !== approved.beaconGenesisValidatorsRoot || BigInt(beacon.genesisBlock.genesis_time) !== approved.beaconGenesisTime ||
        approved.forks.some(fork => BigInt(fork.name === 'phase0' ? '0' : spec[fork.name.toUpperCase() + '_FORK_EPOCH']) !== fork.epoch ||
          spec[fork.name === 'phase0' ? 'GENESIS_FORK_VERSION' : fork.name.toUpperCase() + '_FORK_VERSION'] !== fork.version)) {
      throw new Error('HOLD: fixture Beacon identity/forks differ from independent approval');
    }
    publicClient = createPublicClient({ transport: webSocket(process.env.ETH_RPC_URL!, { reconnect: false }) });
    if (BigInt(await publicClient.getChainId()) !== approved.ethereumChainId || (await publicClient.getBlock({ blockNumber: 0n })).hash !== approved.ethereumGenesisHash) {
      throw new Error('HOLD: fixture execution identity differs from independent approval');
    }
    const beaconBlock = await beacon.getBlock(BigInt(state.checkpoints[0][0]));
    let blockNumber = BigInt(beaconBlock.body.execution_payload.block_number), txHash: HexString | undefined;
    for (let blocks = 0; blocks < 64 && !txHash; blocks++, blockNumber--) {
      const block = await publicClient.getBlock({ blockNumber });
      for (const hash of block.transactions) if ((await publicClient.getTransactionReceipt({ hash })).status === 'success') { txHash = hash; break; }
    }
    if (!txHash) throw new Error('No successful receipt in the bounded applied-checkpoint fixture range');
    const receipt = await publicClient.getTransactionReceipt({ hash: txHash }), block = await publicClient.getBlock({ blockNumber: receipt.blockNumber });
    const slot = Number((block.timestamp - approved.beaconGenesisTime) / 12n);
    if (!Number.isSafeInteger(slot)) throw new Error('Fixture receipt slot exceeds supported exact range');
    const endpoint = await historicalProxy.historicalProxy.endpointFor(slot).atBlock(pin).call();
    if (!('ok' in endpoint) || endpoint.ok !== approved.endpoint.programId) throw new Error('HOLD: fixture receipt endpoint differs from approval');
    const finalized = await publicClient.getBlock({ blockTag: 'finalized' });
    if (receipt.blockNumber > finalized.number || receipt.blockHash !== block.hash) throw new Error('HOLD: fixture receipt is not canonical finalized history');
    return { txHash, profile: approved };
  } finally {
    await gearApi.disconnect();
    if (publicClient) (await publicClient.transport.getRpcClient()).close();
  }
};

export default async () => {
  for (const name of ['VARA_WS_RPC', 'ETH_RPC_URL', 'BEACON_RPC_URL', 'HISTORICAL_PROXY_ID', 'VARA_TO_ETH_NONCE', 'VARA_TO_ETH_BLOCK_NUMBER', 'INBOUND_PROOF_PROFILE_PATH']) {
    if (!process.env[name]) throw new Error('Missing owned SDK fixture prerequisite: ' + name);
  }
  const approved = readApprovedFixtureProfile(process.env.INBOUND_PROOF_PROFILE_PATH!, process.env.HISTORICAL_PROXY_ID!);
  if (!fs.existsSync(PATH_TO_BIN)) execFileSync('cargo', ['build', '--locked', '-p', 'js-test', '--release'], { stdio: 'inherit' });
  const { txHash, profile } = await getFixture(approved);
  execFileSync(PATH_TO_BIN, ['eth-to-vara', txHash], { stdio: 'inherit' });
  execFileSync(PATH_TO_BIN, ['vara-to-eth'], { stdio: 'inherit' });
  fs.writeFileSync('test/tmp/inbound-profile.json', JSON.stringify(profile, (_, value) => typeof value === 'bigint' ? value.toString() : value));
  process.env.TX_HASH = txHash;
};
