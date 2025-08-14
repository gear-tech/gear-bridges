import { beforeAll, afterAll, test, expect, describe } from 'vitest';
import { createPublicClient, PublicClient, webSocket } from 'viem';
import { bytesToHex } from '@ethereumjs/util';
import { GearApi } from '@gear-js/api';
import dotenv from 'dotenv';
import * as fs from 'fs';

import { BeaconClient, createBeaconClient, createEthereumClient, EthereumClient } from '../src/ethereum';
import { encodeEthToVaraEvent, CheckpointClient, ProofResult } from '../src/vara';
import { composeProof } from '../src/eth-to-vara/proof-composer';

dotenv.config();

let gearApi: GearApi;
let publicClient: PublicClient;
let beaconClient: BeaconClient;
let ethClient: EthereumClient;
let checkpointClient: CheckpointClient;

const TX_HASH = process.env.TX_HASH! as `0x${string}`;
const RECEIPT_RLP = fs.readFileSync('test/tmp/receipt_rlp', 'utf8');
const PROOF = fs.readFileSync('test/tmp/proof', 'utf8');
const ETH_TO_VARA_EVENT = fs.readFileSync('test/tmp/eth_to_vara_scale', 'utf8');
const CHECKPOINT_CLIENT_ID = process.env.CHECKPOINT_CLIENT_ID! as `0x${string}`;

beforeAll(async () => {
  gearApi = await GearApi.create({ providerAddress: process.env.VARA_WS_RPC });
  publicClient = createPublicClient({ transport: webSocket(process.env.ETH_RPC_URL!) });
  beaconClient = await createBeaconClient(process.env.BEACON_RPC_URL!);
  ethClient = createEthereumClient(publicClient, beaconClient);
  checkpointClient = new CheckpointClient(gearApi, CHECKPOINT_CLIENT_ID);
});

afterAll(async () => {
  await gearApi.disconnect();
});

describe('eth to vara proof', () => {
  let proof: ProofResult;

  test('generate proof', async () => {
    proof = await composeProof(beaconClient, ethClient, checkpointClient, TX_HASH);
  });

  test('receipt rlp should be correct', () => {
    expect(bytesToHex(proof.receiptRlp).replace('0x', '')).toEqual(RECEIPT_RLP);
  });

  test('proof should be correct', () => {
    expect(
      proof.proof
        .map(bytesToHex)
        .map((b) => b.replace('0x', ''))
        .join(''),
    ).toEqual(PROOF);
  });

  test('eth to vara event should match', () => {
    expect(encodeEthToVaraEvent(proof).slice(2)).toEqual(ETH_TO_VARA_EVENT);
  });
});
