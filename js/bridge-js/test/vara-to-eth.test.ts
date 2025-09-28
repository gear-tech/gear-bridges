import { beforeAll, afterAll, test, expect, describe } from 'vitest';
import { encodeFunctionData } from 'viem';
import { GearApi } from '@gear-js/api';
import dotenv from 'dotenv';
import * as fs from 'fs';

import { GearClient } from '../src/vara';
import { messageHash } from '../src/vara-to-eth/util';
import { getProcessMessageArgs, MessageQueueAbi } from '../src/ethereum';

dotenv.config();

const MESSAGE_HASH = fs.readFileSync('test/tmp/vara_to_eth_message_hash', 'utf8');
const ROOT = fs.readFileSync('test/tmp/vara_to_eth_root', 'utf8');
const PROOF = fs.readFileSync('test/tmp/vara_to_eth_proof', 'utf8');
const NUM_LEAVES = fs.readFileSync('test/tmp/vara_to_eth_num_leaves', 'utf8');
const LEAF_INDEX = fs.readFileSync('test/tmp/vara_to_eth_leaf_index', 'utf8');
const PROCESS_MESSAGE_CALL = fs.readFileSync('test/tmp/process_message_calldata', 'utf8');

const NONCE = BigInt(process.env.VARA_TO_ETH_NONCE!);
const BLOCK_NUMBER = Number(process.env.VARA_TO_ETH_BLOCK_NUMBER!);

let gearApi: GearApi;
let gearClient: GearClient;
// let publicClient: PublicClient;

beforeAll(async () => {
  gearApi = await GearApi.create({ providerAddress: process.env.VARA_WS_RPC });
  gearClient = new GearClient(gearApi);
});

afterAll(async () => {
  await gearApi.disconnect();
});

describe('VaraToEth', () => {
  test('message hash', async () => {
    const msg = await gearClient.findMessageQueuedEvent(BLOCK_NUMBER, NONCE);

    if (!msg) {
      throw new Error('Message not found');
    }

    const hash = messageHash(msg);

    expect(hash.slice(2)).toEqual(MESSAGE_HASH);
  });

  test('merkle proof', async () => {
    const msg = await gearClient.findMessageQueuedEvent(BLOCK_NUMBER, NONCE);

    if (!msg) {
      throw new Error('Message not found');
    }

    const merkleProof = await gearClient.fetchMerkleProof(BLOCK_NUMBER, messageHash(msg));

    expect(merkleProof.leafIndex.toString()).toEqual(LEAF_INDEX);
    expect(merkleProof.numLeaves.toString()).toEqual(NUM_LEAVES);
    expect(merkleProof.root.slice(2)).toEqual(ROOT);
    expect(merkleProof.proof.map((i) => i.slice(2)).join('')).toEqual(PROOF);
  });

  test('process message call', async () => {
    const msg = await gearClient.findMessageQueuedEvent(BLOCK_NUMBER, NONCE);

    if (!msg) {
      throw new Error('Message not found');
    }

    const merkleProof = await gearClient.fetchMerkleProof(BLOCK_NUMBER, messageHash(msg));

    const data = encodeFunctionData({
      abi: MessageQueueAbi,
      functionName: 'processMessage',
      args: getProcessMessageArgs(BigInt(BLOCK_NUMBER), msg, merkleProof),
    });

    expect(data.slice(2)).toEqual(PROCESS_MESSAGE_CALL);
  });
});
