import { beforeAll, afterAll, test, expect, describe } from 'vitest';
import { createPublicClient, PublicClient, webSocket } from 'viem';
import { GearApi } from '@gear-js/api';
import * as fs from 'fs';
import dotenv from 'dotenv';

dotenv.config();

import { BeaconClient, composeProof, createBeaconClient, createEthereumClient, EthereumClient } from '../src';

let gearApi: GearApi;
let publicClient: PublicClient;
let beaconClient: BeaconClient;
let ethClient: EthereumClient;
const TX_HASH = process.env.TX_HASH! as `0x${string}`;
const RECEIPT_RLP = fs.readFileSync('test/tmp/receipt_rlp.txt', 'utf8');
const PROOF = fs.readFileSync('test/tmp/proof.txt', 'utf8');

beforeAll(async () => {
  gearApi = await GearApi.create({ providerAddress: process.env.GEAR_RPC_URL });
  publicClient = createPublicClient({ transport: webSocket(process.env.ETH_RPC_URL!) });
  beaconClient = await createBeaconClient(process.env.BEACON_RPC_URL!);
  ethClient = await createEthereumClient(publicClient, beaconClient);
});

afterAll(async () => {
  await gearApi.disconnect();
});

describe('eth to vara proof', () => {
  test('receipt rlp should be correct', async () => {
    const result = await composeProof(beaconClient, ethClient, TX_HASH);

    expect(result.receipt_rlp).toEqual(RECEIPT_RLP);
  });

  test('proof should be correct', async () => {
    const result = await composeProof(beaconClient, ethClient, TX_HASH);

    expect(result.proof).toEqual(PROOF);
  });
});
