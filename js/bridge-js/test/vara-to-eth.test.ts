import { beforeAll, afterAll, test, expect, describe, vi } from 'vitest';
import { encodeFunctionData, encodeEventTopics, encodeAbiParameters, parseAbiParameters, PublicClient, WalletClient, Account, hexToBytes } from 'viem';
import { GearApi, HexString } from '@gear-js/api';
import dotenv from 'dotenv';
import * as fs from 'fs';

import { GearClient, decodeEthBridgeMessageResponse } from '../src/vara/index.js';
import { Proof, VaraMessage } from '../src/vara/types.js';
import { messageHash } from '../src/vara-to-eth/util.js';
import { prepareVaraToEthRelay } from '../src/vara-to-eth/relayer.js';
import { getProcessMessageArgs, MessageQueueAbi } from '../src/ethereum/index.js';
import { MessageQueueClient, validateFinalizedMessageReceipt, BridgedEventAbi, OutboundEffect } from '../src/ethereum/message-queue.js';

dotenv.config();
const hash = (value: number): HexString => ('0x' + value.toString(16).padStart(64, '0')) as HexString;
const address = (value: number): HexString => ('0x' + value.toString(16).padStart(40, '0')) as HexString;
const scalar = (value: bigint) => ({ toBigInt: () => value });
const pin = (value: HexString) => ({ toHex: () => value, eq: (other: { toHex(): string } | string) => value === (typeof other === 'string' ? other : other.toHex()) });
const message: VaraMessage = { nonce: 0n, source: hexToBytes(hash(3)), destination: hexToBytes(address(4)), payload: Uint8Array.of(0, 255, 1) };
const proof: Proof = { root: hash(55), proof: [], numLeaves: 1n, leafIndex: 0n };

function receiptFixture(token = false) {
  const txHash = hash(7), queue = address(1), sender = address(2), rootBlock = 12n;
  const effect: OutboundEffect = token ? { kind: 'token', managerAddress: address(4), sourceActorId: hash(3),
    token: address(10), sender: hash(8), receiver: address(9), amount: 12n } : { kind: 'application' };
  const originalMessage = token ? { ...message, payload: hexToBytes((hash(8) + address(9).slice(2) + address(10).slice(2) + hash(12).slice(2)) as HexString) } : message;
  const expectedHash = messageHash(originalMessage);
  const input = encodeFunctionData({ abi: MessageQueueAbi, functionName: 'processMessage', args: getProcessMessageArgs(rootBlock, originalMessage, proof) });
  const state = { finalized: 6n, canonicalHash: hash(5), receiptStatus: 'success', processed: true, storedRoot: proof.root,
    eventHash: expectedHash, eventNonce: message.nonce, eventDestination: address(4), eventBlock: rootBlock,
    logAddress: queue, input, duplicate: false, effectFrom: hash(8), effectTo: address(9), effectToken: address(10), effectAmount: 12n,
    effectAddress: address(4), effectHash: txHash, effectMissing: false, effectDuplicate: false };
  const transaction = () => ({ hash: txHash, blockHash: hash(5), blockNumber: 5n, transactionIndex: 0, from: sender, to: queue, value: 0n, input: state.input });
  const log = () => ({ address: state.logAddress, transactionHash: txHash, blockHash: hash(5), blockNumber: 5n,
    transactionIndex: 0, logIndex: 0, removed: false,
    topics: encodeEventTopics({ abi: MessageQueueAbi, eventName: 'MessageProcessed' }),
    data: encodeAbiParameters(parseAbiParameters('uint256, bytes32, uint256, address'),
      [state.eventBlock, state.eventHash, state.eventNonce, state.eventDestination]),
  });
  const bridged = () => ({ ...log(), address: state.effectAddress, transactionHash: state.effectHash, logIndex: 1,
    topics: encodeEventTopics({ abi: BridgedEventAbi, eventName: 'Bridged',
      args: { from: state.effectFrom, to: state.effectTo, token: state.effectToken } }),
    data: encodeAbiParameters(parseAbiParameters('uint256'), [state.effectAmount]) });
  const logs = () => [log(), ...(state.duplicate ? [{ ...log(), logIndex: 1 }] : []),
    ...(token && !state.effectMissing ? [bridged(), ...(state.effectDuplicate ? [{ ...bridged(), logIndex: 2 }] : [])] : [])];
  const client = {
    pollingInterval: 1,
    getTransactionReceipt: async () => ({ transactionHash: txHash, blockHash: hash(5), blockNumber: 5n, transactionIndex: 0,
      status: state.receiptStatus, logs: logs() }),
    getTransaction: async () => transaction(),
    getBlock: async ({ blockTag }: { blockTag?: string }) => blockTag === 'finalized'
      ? { number: state.finalized, hash: hash(Number(state.finalized)) } : { number: 5n, hash: state.canonicalHash },
    readContract: async ({ functionName }: { functionName: string }) => functionName === 'isProcessed' ? state.processed : state.storedRoot,
  } as unknown as PublicClient;
  return { state, params: { ethereumPublicClient: client, messageQueueAddress: queue, transactionHash: txHash,
    blockNumber: rootBlock, message: originalMessage, proof, sender, expectedEffect: effect, deadline: Date.now() + 1000 } };
}

function rootsFixture() {
  const roots = new Map([[8n, hash(8)], [10n, hash(10)], [12n, proof.root]]);
  const api = {
    blocks: {
      getFinalizedHead: async () => pin(hash(20)),
      getBlockHash: async (height: bigint | number) => pin(hash(Number(height))),
      getEvents: async () => [{ event: { section: 'gearEthBridge', method: 'MessageQueued', data: [{
        nonce: scalar(0n), source: pin(hash(3)), destination: pin(address(4)), payload: { toU8a: () => message.payload },
      }] } }],
    },
    rpc: { chain: { getHeader: async () => ({ number: scalar(20n) }) } },
    ethBridge: { merkleProof: async (_hash: HexString, rootPin: { toHex(): HexString }) => ({
      root: pin(BigInt(rootPin.toHex()) === 12n ? proof.root : hash(99)), proof: [], number_of_leaves: scalar(1n), leaf_index: scalar(0n),
    }) },
  } as unknown as GearApi;
  const client = {
    pollingInterval: 1,
    getBlock: async () => ({ number: 2n, hash: hash(2) }),
    getLogs: async () => [{ args: { blockNumber: 12n, merkleRoot: proof.root } }, { args: { blockNumber: 10n, merkleRoot: hash(10) } }],
    readContract: async ({ args }: { args: readonly bigint[] }) => roots.get(args[0]) ?? hash(0),
  } as unknown as PublicClient;
  return { roots, params: { gearApi: api, ethereumPublicClient: client, messageQueueAddress: address(1), blockNumber: 8n,
    nonce: 0n, wait: false, deadline: Date.now() + 1000 } };
}

describe('SDK outbound acceptance', () => {
  test('decodes only the exact tagged 77-byte response, preserving nonce zero', () => {
    const bytes = new Uint8Array(77);
    new DataView(bytes.buffer).setUint32(1, 7, true);
    bytes.fill(0x33, 5, 37);
    new DataView(bytes.buffer).setBigUint64(69, 9n, true);
    expect(decodeEthBridgeMessageResponse(bytes)).toEqual({ blockNumber: 7n, hash: '0x' + '33'.repeat(32), nonce: 0n, queueId: 9n });
    expect(() => decodeEthBridgeMessageResponse(bytes.subarray(1))).toThrow();
    expect(() => decodeEthBridgeMessageResponse(new Uint8Array([...bytes, 0]))).toThrow();
    bytes[0] = 1;
    expect(() => decodeEthBridgeMessageResponse(bytes)).toThrow();
  });

  test('an unrelated earlier stored root does not mask a later original-message proof across a handover', async () => {
    const fixture = rootsFixture();
    const prepared = await prepareVaraToEthRelay(fixture.params);
    expect(prepared.blockNumber).toBe(12n);
    expect(prepared.message.nonce).toBe(0n);
    expect(prepared.proof.root).toBe(proof.root);
    expect(prepared.args).toEqual(getProcessMessageArgs(12n, message, proof));
  });

  test('unavailable old proofs do not create a replacement claim or reset an expired deadline', async () => {
    const fixture = rootsFixture();
    fixture.roots.delete(12n);
    await expect(prepareVaraToEthRelay(fixture.params)).rejects.toThrow('No authenticated stored-root claim');
    fixture.params.deadline = Date.now() - 1;
    await expect(prepareVaraToEthRelay(fixture.params)).rejects.toThrow('deadline expired');
  });

  test('returns canonical finalized configured-queue delivery evidence for nonce zero, including original-result reconciliation', async () => {
    const fixture = receiptFixture();
    const result = await validateFinalizedMessageReceipt(fixture.params);
    expect(result.success).toBe(true);
    expect(result.messageNonce).toBe(0n);
    expect(result.messageHash).toBe(messageHash(message));
    expect(result.messageDestination).toBe(address(4));
    expect(result.receiptBlockHash).toBe(hash(5));
    expect(result.receiptBlockNumber).toBe(5n);
    expect(result.transactionHash).toBe(hash(7));
    expect((await validateFinalizedMessageReceipt(fixture.params)).transactionHash).toBe(hash(7));
  });

  test('token completion requires the exact manager effect in the original finalized receipt on resume', async () => {
    const fixture = receiptFixture(true);
    expect((await validateFinalizedMessageReceipt(fixture.params)).transactionHash).toBe(hash(7));
    expect((await validateFinalizedMessageReceipt(fixture.params)).transactionHash).toBe(hash(7));
  });

  test.each(['missing', 'duplicate', 'sender', 'receiver', 'token', 'amount', 'manager', 'other-transaction', 'source-payload'])(
    'holds token completion on %s same-receipt effect mismatch', async (kind) => {
      const fixture = receiptFixture(true);
      if (kind === 'missing') fixture.state.effectMissing = true;
      if (kind === 'duplicate') fixture.state.effectDuplicate = true;
      if (kind === 'sender') fixture.state.effectFrom = hash(99);
      if (kind === 'receiver') fixture.state.effectTo = address(99);
      if (kind === 'token') fixture.state.effectToken = address(99);
      if (kind === 'amount') fixture.state.effectAmount = 99n;
      if (kind === 'manager') fixture.state.effectAddress = address(99);
      if (kind === 'other-transaction') fixture.state.effectHash = hash(99);
      if (kind === 'source-payload' && fixture.params.expectedEffect.kind === 'token') {
        fixture.params.expectedEffect = { ...fixture.params.expectedEffect, amount: 99n };
      }
      await expect(validateFinalizedMessageReceipt(fixture.params)).rejects.toThrow();
    });


  test('timed-out local EVM signing cannot trigger a later raw submission', async () => {
    vi.useFakeTimers();
    const fixture = receiptFixture();
    let finishSigning!: (bytes: HexString) => void, signingStarted!: () => void;
    const signature = new Promise<HexString>((resolve) => { finishSigning = resolve; });
    const started = new Promise<void>((resolve) => { signingStarted = resolve; });
    let broadcasts = 0;
    const wallet = { prepareTransactionRequest: async () => ({}),
      signTransaction: async () => { signingStarted(); return signature; },
      sendRawTransaction: async () => { broadcasts++; return hash(7); },
      writeContract: async () => { await wallet.signTransaction(); return wallet.sendRawTransaction(); } };
    try {
      const client = new MessageQueueClient(fixture.params.messageQueueAddress,
        { ...fixture.params.ethereumPublicClient, simulateContract: async () => ({}) } as unknown as PublicClient,
        wallet as unknown as WalletClient, { address: fixture.params.sender, type: 'local', signTransaction: wallet.signTransaction } as unknown as Account);
      const pending = client.processMessage(12n, message, proof, { kind: 'application' }, undefined, Date.now() + 1000);
      await started;
      await vi.advanceTimersByTimeAsync(1001);
      const result = await pending;
      expect(result.success).toBe(false);
      expect(result.transactionHash).toBe('0x');
      expect(result.error).toContain('deadline expired');
      finishSigning('0x01');
      await Promise.resolve();
      await Promise.resolve();
      expect(broadcasts).toBe(0);
    } finally { vi.useRealTimers(); }
  });

  test('an admitted remote-wallet timeout is HOLD, not cancellation or completed delivery', async () => {
    vi.useFakeTimers();
    const fixture = receiptFixture();
    let confirm!: () => void, admitted!: () => void;
    const confirmation = new Promise<void>((resolve) => { confirm = resolve; });
    const admission = new Promise<void>((resolve) => { admitted = resolve; });
    let broadcast = false;
    const wallet = { writeContract: async () => { admitted(); await confirmation; broadcast = true; return hash(7); } };
    try {
      const client = new MessageQueueClient(fixture.params.messageQueueAddress,
        { ...fixture.params.ethereumPublicClient, simulateContract: async () => ({}) } as unknown as PublicClient,
        wallet as unknown as WalletClient, { address: fixture.params.sender, type: 'json-rpc' } as Account);
      const pending = client.processMessage(12n, message, proof, { kind: 'application' }, undefined, Date.now() + 1000);
      await admission;
      await vi.advanceTimersByTimeAsync(1001);
      const result = await pending;
      expect(result.success).toBe(false);
      expect(result.transactionHash).toBe('0x');
      expect(result.error).toContain('HOLD:');
      expect(result.error).toContain('may still broadcast');
      confirm();
      await Promise.resolve();
      await Promise.resolve();
      expect(broadcast).toBe(true);
    } finally { vi.useRealTimers(); }
  });



  test('a stalled historical proof RPC cannot extend the original preparation deadline', async () => {
    const fixture = rootsFixture();
    fixture.params.gearApi.ethBridge.merkleProof = async () => new Promise<never>(() => {});
    fixture.params.deadline = Date.now() + 30;
    await expect(prepareVaraToEthRelay(fixture.params)).rejects.toThrow('deadline expired');
  });

  test.each(['hash', 'nonce', 'destination', 'root-block', 'queue', 'duplicate', 'calldata', 'state', 'root-state', 'canonical', 'reverted'])(
    'rejects mismatched %s receipt evidence', async (kind) => {
      const fixture = receiptFixture();
      if (kind === 'hash') fixture.state.eventHash = hash(99);
      if (kind === 'nonce') fixture.state.eventNonce = 1n;
      if (kind === 'destination') fixture.state.eventDestination = address(99);
      if (kind === 'root-block') fixture.state.eventBlock = 99n;
      if (kind === 'queue') fixture.state.logAddress = address(99);
      if (kind === 'duplicate') fixture.state.duplicate = true;
      if (kind === 'calldata') fixture.state.input = '0x1234';
      if (kind === 'state') fixture.state.processed = false;
      if (kind === 'root-state') fixture.state.storedRoot = hash(99);
      if (kind === 'canonical') fixture.state.canonicalHash = hash(99);
      if (kind === 'reverted') fixture.state.receiptStatus = 'reverted';
      await expect(validateFinalizedMessageReceipt(fixture.params)).rejects.toThrow();
    });

  test('a successful first-mined receipt cannot complete before original canonical finality', async () => {
    const fixture = receiptFixture();
    fixture.state.finalized = 4n;
    fixture.params.deadline = Date.now() + 30;
    await expect(validateFinalizedMessageReceipt(fixture.params)).rejects.toThrow('deadline expired');
  });
});

describe('VaraToEth', () => {
  let gearApi: GearApi;
  let gearClient: GearClient;
  let expected: string[];
  let nonce: bigint, blockNumber: number;
  beforeAll(async () => {
    expected = ['vara_to_eth_message_hash', 'vara_to_eth_root', 'vara_to_eth_proof', 'vara_to_eth_num_leaves', 'vara_to_eth_leaf_index', 'process_message_calldata']
      .map((name) => fs.readFileSync('test/tmp/' + name, 'utf8'));
    nonce = BigInt(process.env.VARA_TO_ETH_NONCE!);
    blockNumber = Number(process.env.VARA_TO_ETH_BLOCK_NUMBER!);
    gearApi = await GearApi.create({ providerAddress: process.env.VARA_WS_RPC });
    gearClient = new GearClient(gearApi);
  });
  afterAll(async () => { if (gearApi) await gearApi.disconnect(); });
  test('message hash', async () => {
    const msg = await gearClient.findMessageQueuedEvent(blockNumber, nonce);
    if (!msg) throw new Error('Message not found');
    expect(messageHash(msg).slice(2)).toEqual(expected[0]);
  });
  test('merkle proof', async () => {
    const msg = await gearClient.findMessageQueuedEvent(blockNumber, nonce);
    if (!msg) throw new Error('Message not found');
    const merkleProof = await gearClient.fetchMerkleProof(blockNumber, messageHash(msg));
    expect(merkleProof.leafIndex.toString()).toEqual(expected[4]);
    expect(merkleProof.numLeaves.toString()).toEqual(expected[3]);
    expect(merkleProof.root.slice(2)).toEqual(expected[1]);
    expect(merkleProof.proof.map((item) => item.slice(2)).join('')).toEqual(expected[2]);
  });
  test('process message call', async () => {
    const msg = await gearClient.findMessageQueuedEvent(blockNumber, nonce);
    if (!msg) throw new Error('Message not found');
    const merkleProof = await gearClient.fetchMerkleProof(blockNumber, messageHash(msg));
    const data = encodeFunctionData({ abi: MessageQueueAbi, functionName: 'processMessage', args: getProcessMessageArgs(BigInt(blockNumber), msg, merkleProof) });
    expect(data.slice(2)).toEqual(expected[5]);
  });
});
