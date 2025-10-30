import { createPublicClient, webSocket } from 'viem';
import { execSync } from 'child_process';
import * as path from 'path';
import * as fs from 'fs';
import dotenv from 'dotenv';
import { GearApi } from '@gear-js/api';
import { CheckpointClient, EthEventsClient, HistoricalProxyClient } from '../../src/vara';
import { createBeaconClient } from '../../src/ethereum';

dotenv.config();

const TARGET_DIR = '../../target';
const PATH_TO_BIN = path.join(TARGET_DIR, 'release/js-test');

const getTxHash = async () => {
  const gearApi = await GearApi.create({ providerAddress: process.env.VARA_WS_RPC });
  const historicalProxy = new HistoricalProxyClient(gearApi, process.env.HISTORICAL_PROXY_ID as `0x${string}`);
  const endpoints = await historicalProxy.historicalProxy.endpoints().call();
  const latest = endpoints.sort(([slotA], [slotB]) => slotB - slotA)[0][1];
  const ethEvents = new EthEventsClient(gearApi, latest);
  const checkpointClient = new CheckpointClient(
    gearApi,
    await ethEvents.ethereumEventClient.checkpointLightClientAddress().call(),
  );

  const slot = await checkpointClient.serviceState.getLatestSlot();

  const beaconClient = await createBeaconClient(process.env.BEACON_RPC_URL!);

  const beaconBlock = await beaconClient.getBlock(BigInt(slot.checkpoints[0][0]));

  const publicClient = createPublicClient({ transport: webSocket(process.env.ETH_RPC_URL!) });

  let block = await publicClient.getBlock({ blockNumber: BigInt(beaconBlock.body.execution_payload.block_number) });

  if (block.transactions.length == 0) {
    while (block.transactions.length == 0) {
      block = await publicClient.getBlock({ blockNumber: block.number - 1n });
    }
  }

  return block.transactions[0];
};

export default async () => {
  if (!fs.existsSync(PATH_TO_BIN)) {
    execSync(`cargo build -p js-test --release`, { stdio: 'inherit' });
  }

  const txHash = await getTxHash();

  execSync(`${PATH_TO_BIN} eth-to-vara ${txHash}`, { stdio: 'inherit' });
  execSync(`${PATH_TO_BIN} vara-to-eth`, { stdio: 'inherit' });

  process.env['TX_HASH'] = txHash;
};
