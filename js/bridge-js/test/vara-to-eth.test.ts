import { beforeAll, afterAll, test, expect, describe, vi } from 'vitest';
import { encodeFunctionData, encodeEventTopics, encodeAbiParameters, parseAbiParameters, PublicClient, WalletClient, Account, hexToBytes, ContractFunctionRevertedError, encodeErrorResult, keccak256 } from 'viem';
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

function receiptFixture(token = false, txHash = hash(7)) {
  const queue = address(1), sender = address(2), rootBlock = 12n;
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
    rpc: { chain: { getHeader: async (at: HexString | { toHex(): HexString }) => ({ number: scalar(BigInt(typeof at === 'string' ? at : at.toHex())) }) } },
    ethBridge: { merkleProof: async (_hash: HexString, rootPin: HexString) => ({
      root: pin(BigInt(rootPin) === 12n ? proof.root : hash(99)), proof: [], number_of_leaves: scalar(1n), leaf_index: scalar(0n),
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

const rootLog = (blockNumber: bigint, merkleRoot = hash(Number(blockNumber)), removed = false) =>
  ({ args: { blockNumber, merkleRoot }, removed });

function availabilityFixture() {
  const unwatch = vi.fn();
  type Watch = { fromBlock: bigint; onLogs(logs: ReturnType<typeof rootLog>[]): void; onError(error: Error): void };
  let watching!: (watch: Watch) => void;
  const watched = new Promise<Watch>((resolve) => { watching = resolve; });
  const client = {
    readContract: vi.fn(async (_request: { functionName: string }) => hash(0)),
    getBlockNumber: vi.fn(async () => 100n),
    getLogs: vi.fn(async (_range: { fromBlock: bigint; toBlock: bigint }) => [] as ReturnType<typeof rootLog>[]),
    watchContractEvent: vi.fn((watch: Watch) => { watching(watch); return unwatch; }),
  };
  return { client, unwatch, watched, queue: new MessageQueueClient(address(1), client as unknown as PublicClient) };
}

describe('SDK advisory root availability', () => {
  test('returns the exact finalized stored root without reading history or watching', async () => {
    const { client, queue } = availabilityFixture();
    client.readContract.mockResolvedValue(hash(10));
    expect(await queue.waitForMerkleRoot(10n)).toEqual(rootLog(10n).args);
    expect(client.readContract).toHaveBeenCalledWith(expect.objectContaining({
      functionName: 'getMerkleRoot', args: [10n], blockTag: 'finalized',
    }));
    expect(client.getBlockNumber).not.toHaveBeenCalled();
    expect(client.getLogs).not.toHaveBeenCalled();
    expect(client.watchContractEvent).not.toHaveBeenCalled();
  });

  test('returns the lowest eligible source height in the first recent page, not the global minimum', async () => {
    const { client, queue } = availabilityFixture();
    client.getBlockNumber.mockResolvedValue(10_000n);
    client.getLogs.mockImplementation(async ({ fromBlock }) =>
      fromBlock >= 8001n ? [rootLog(30n), rootLog(15n), rootLog(20n)] : [rootLog(12n)]);
    expect(await queue.waitForMerkleRoot(10n, 500n)).toEqual(rootLog(15n).args);
    expect(client.getLogs.mock.calls.map(([{ fromBlock, toBlock }]) => [fromBlock, toBlock]))
      .toEqual([[8001n, 10_000n]]);
    expect(client.watchContractEvent).not.toHaveBeenCalled();
  });

  test.each([undefined, 500n])('walks bounded recent-first pages through the inclusive floor %s', async (floor) => {
    const { client, queue } = availabilityFixture();
    client.getBlockNumber.mockResolvedValue(4500n);
    client.getLogs.mockImplementation(async ({ fromBlock }) =>
      fromBlock === (floor ?? 0n) ? [rootLog(12n)] : []);
    expect(await queue.waitForMerkleRoot(10n, floor)).toEqual(rootLog(12n).args);
    expect(client.getLogs.mock.calls.map(([{ fromBlock, toBlock }]) => [fromBlock, toBlock]))
      .toEqual([[2501n, 4500n], [501n, 2500n], [floor ?? 0n, 500n]]);
  });

  test('finds a late backfill below maximum source progress without treating that maximum as a cursor', async () => {
    const { client, queue } = availabilityFixture();
    client.readContract.mockImplementation(async ({ functionName }) =>
      functionName === 'getMerkleRoot' ? hash(0) : hash(100));
    client.getLogs.mockResolvedValue([rootLog(12n)]);
    expect(await queue.waitForMerkleRoot(10n)).toEqual(rootLog(12n).args);
    expect(client.readContract).toHaveBeenCalledTimes(1);
    expect(client.watchContractEvent).not.toHaveBeenCalled();
  });

  test.each([100n, 4100n])('catches handoff publication inclusively through new head %s', async (head) => {
    const { client, queue, unwatch } = availabilityFixture();
    client.getBlockNumber.mockResolvedValueOnce(100n).mockResolvedValueOnce(head);
    client.getLogs.mockImplementation(async ({ fromBlock, toBlock }) =>
      client.watchContractEvent.mock.calls.length && fromBlock <= 100n && toBlock >= 100n ? [rootLog(12n)] : []);
    expect(await queue.waitForMerkleRoot(10n, 50n)).toEqual(rootLog(12n).args);
    expect(client.watchContractEvent).toHaveBeenCalledWith(expect.objectContaining({ fromBlock: 100n }));
    expect(client.getLogs.mock.calls.map(([{ fromBlock, toBlock }]) => [fromBlock, toBlock]))
      .toEqual(head === 100n ? [[50n, 100n], [100n, 100n]] :
        [[50n, 100n], [2101n, 4100n], [101n, 2100n], [100n, 100n]]);
    expect(unwatch).toHaveBeenCalledTimes(1);
  });

  test('ignores removed, zero and below-target roots in history and live batches', async () => {
    const { client, queue, watched, unwatch } = availabilityFixture();
    const ignored = [rootLog(11n, hash(11), true), rootLog(12n, hash(0)), rootLog(9n)];
    client.getLogs.mockResolvedValue(ignored);
    const pending = queue.waitForMerkleRoot(10n);
    const watch = await watched;
    watch.onLogs(ignored);
    expect(unwatch).not.toHaveBeenCalled();
    watch.onLogs([...ignored, rootLog(20n), rootLog(15n)]);
    expect(await pending).toEqual(rootLog(15n).args);
    watch.onLogs([rootLog(10n)]);
    watch.onError(new Error('late watcher error'));
    expect(unwatch).toHaveBeenCalledTimes(1);
  });

  test('cleans up once when logs arrive synchronously before the unwatch function is returned', async () => {
    const { client, queue, unwatch } = availabilityFixture();
    client.watchContractEvent.mockImplementation((watch) => {
      watch.onLogs([rootLog(12n)]);
      watch.onError(new Error('later error must not replace success'));
      return unwatch;
    });
    expect(await queue.waitForMerkleRoot(10n)).toEqual(rootLog(12n).args);
    expect(unwatch).toHaveBeenCalledTimes(1);
    expect(client.getBlockNumber).toHaveBeenCalledTimes(1);
  });

  test('a live result wins over a later failing catchup RPC without repeated cleanup', async () => {
    const { client, queue, watched, unwatch } = availabilityFixture();
    let started!: () => void, fail!: (error: Error) => void;
    const admission = new Promise<void>((resolve) => { started = resolve; });
    const catchup = new Promise<ReturnType<typeof rootLog>[]>((_, reject) => { fail = reject; });
    client.getLogs.mockResolvedValueOnce([]).mockImplementationOnce(async () => {
      started();
      return catchup;
    });
    const pending = queue.waitForMerkleRoot(10n);
    await admission;
    const watch = await watched;
    watch.onLogs([rootLog(12n)]);
    expect(await pending).toEqual(rootLog(12n).args);
    fail(new Error('late catchup error'));
    await Promise.resolve();
    await Promise.resolve();
    expect(unwatch).toHaveBeenCalledTimes(1);
  });

  test.each(['history', 'catchup-head', 'catchup-logs', 'watch-setup', 'sync-watch', 'watch'])(
    'keeps %s errors visible and unwatches any installed watcher exactly once', async (stage) => {
      const { client, queue, watched, unwatch } = availabilityFixture();
      const error = new Error(stage + ' unavailable');
      if (stage === 'history') client.getLogs.mockRejectedValueOnce(error);
      if (stage === 'catchup-head') client.getBlockNumber.mockResolvedValueOnce(100n).mockRejectedValueOnce(error);
      if (stage === 'catchup-logs') client.getLogs.mockResolvedValueOnce([]).mockRejectedValueOnce(error);
      if (stage === 'watch-setup') client.watchContractEvent.mockImplementation(() => { throw error; });
      if (stage === 'sync-watch') client.watchContractEvent.mockImplementation((watch) => {
        watch.onError(error);
        watch.onLogs([rootLog(12n)]);
        return unwatch;
      });
      const pending = queue.waitForMerkleRoot(10n);
      const rejection = expect(pending).rejects.toBe(error);
      if (stage === 'watch') {
        const watch = await watched;
        watch.onError(error);
        watch.onError(error);
        watch.onLogs([rootLog(12n)]);
      }
      await rejection;
      expect(unwatch).toHaveBeenCalledTimes(stage === 'history' || stage === 'watch-setup' ? 0 : 1);
    });
});

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

  test('recent finalized roots are tried before walking millions of empty historical blocks', async () => {
    const fixture = rootsFixture();
    fixture.roots.delete(8n);
    fixture.params.ethereumPublicClient.getBlock = async () => ({ number: 2_000_000n, hash: hash(2) }) as never;
    const ranges: [bigint, bigint][] = [];
    fixture.params.ethereumPublicClient.getLogs = async ({ fromBlock, toBlock } = {}) => {
      ranges.push([fromBlock as bigint, toBlock as bigint]);
      if ((fromBlock as bigint) >= 1_998_001n) return [{ args: { blockNumber: 12n, merkleRoot: proof.root } }] as never;
      return [] as never;
    };
    expect((await prepareVaraToEthRelay(fixture.params)).blockNumber).toBe(12n);
    expect(ranges).toEqual([[1_998_001n, 2_000_000n]]);
  });

  test('expired discovery stops issuing pages after an already admitted log RPC returns', async () => {
    vi.useFakeTimers();
    const fixture = rootsFixture();
    fixture.roots.clear();
    fixture.params.ethereumPublicClient.getBlock = async () => ({ number: 2_000_000n, hash: hash(2) }) as never;
    let finish!: (logs: never[]) => void, started!: () => void;
    const admission = new Promise<void>((resolve) => { started = resolve; });
    let calls = 0;
    fixture.params.ethereumPublicClient.getLogs = async () => {
      calls++;
      if (calls === 1) { started(); return new Promise((resolve) => { finish = resolve; }) as never; }
      return [] as never;
    };
    fixture.params.deadline = Date.now() + 30;
    try {
      const rejection = expect(prepareVaraToEthRelay(fixture.params)).rejects.toThrow('deadline expired');
      await admission;
      await vi.advanceTimersByTimeAsync(31);
      await rejection;
      finish([]);
      await vi.advanceTimersByTimeAsync(10);
      expect(calls).toBe(1);
    } finally { vi.useRealTimers(); }
  });

  test.each(['verified', 'missing-code', 'archive-unavailable', 'noncanonical'])(
    '%s deployment hint never hides an older valid stored-root claim', async (kind) => {
      const fixture = rootsFixture();
      fixture.roots.delete(8n);
      const params = { ...fixture.params, messageQueueDeploymentBlock: 9000n };
      let canonicalReads = 0;
      params.ethereumPublicClient.getBlock = async ({ blockNumber } = {}) => ({
        number: blockNumber ?? 10_000n,
        hash: kind === 'noncanonical' && blockNumber === 9000n && ++canonicalReads > 1 ? hash(99) : hash(Number(blockNumber ?? 10_000n)),
      }) as never;
      params.ethereumPublicClient.getBytecode = async ({ blockNumber }) => {
        if (kind === 'archive-unavailable') throw new Error('Historical code unavailable');
        return blockNumber === 9000n && kind !== 'missing-code' ? '0x6000' : '0x';
      };
      const ranges: [bigint, bigint][] = [];
      params.ethereumPublicClient.getLogs = async ({ fromBlock, toBlock } = {}) => {
        ranges.push([fromBlock as bigint, toBlock as bigint]);
        return (fromBlock as bigint) <= 8000n && (toBlock as bigint) >= 8000n
          ? [{ args: { blockNumber: 12n, merkleRoot: proof.root } }] as never : [] as never;
      };
      expect((await prepareVaraToEthRelay(params)).blockNumber).toBe(12n);
      if (kind === 'verified') expect(ranges).toEqual([[9000n, 10_000n], [7000n, 8999n]]);
      else expect(ranges).toEqual([[8001n, 10_000n], [6001n, 8000n]]);
    });

  test('discovery cannot use an indexer-like log whose root is absent from finalized queue storage', async () => {
    const fixture = rootsFixture();
    fixture.roots.delete(8n);
    fixture.roots.delete(12n);
    await expect(prepareVaraToEthRelay(fixture.params)).rejects.toThrow('No authenticated stored-root claim');
  });

  test('source and Ethereum canonical pins remain mandatory with recent-first discovery', async () => {
    for (const network of ['source', 'ethereum']) {
      const fixture = rootsFixture();
      fixture.roots.delete(8n);
      let sourceReads = 0;
      if (network === 'source') fixture.params.gearApi.blocks.getBlockHash = async (height) => pin(hash(Number(height) === 8 && ++sourceReads > 1 ? 99 : Number(height))) as never;
      else fixture.params.ethereumPublicClient.getBlock = async ({ blockTag } = {}) => ({ number: 2n, hash: blockTag === 'finalized' ? hash(2) : hash(99) }) as never;
      await expect(prepareVaraToEthRelay(fixture.params)).rejects.toThrow('Canonical finalized source or queue history changed');
    }
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


  test.each(['matures', 'deadline', 'root', 'anchor', 'other', 'disabled'])('unsigned maturity waiting: %s', async (kind) => {
    vi.useFakeTimers();
    const fixture = receiptFixture(false, keccak256('0x01'));
    const sign = vi.fn(async () => '0x01'), broadcast = vi.fn(async () => fixture.params.transactionHash);
    const simulated: unknown[] = [];
    const client = { ...fixture.params.ethereumPublicClient,
      getBlock: async ({ blockTag, blockNumber }: { blockTag?: string; blockNumber?: bigint }) =>
        blockTag === 'finalized' ? { number: 6n, hash: hash(6) } :
          { number: blockNumber, hash: blockNumber === 6n && kind === 'anchor' ? hash(99) : hash(Number(blockNumber)) },
      readContract: async ({ functionName }: { functionName: string }) => functionName === 'isProcessed' ? true : kind === 'root' ? hash(99) : proof.root,
      simulateContract: async ({ args }: { args: unknown }) => {
        simulated.push(args);
        if (simulated.length === 1 || kind === 'deadline') throw new ContractFunctionRevertedError({
          abi: MessageQueueAbi, functionName: 'processMessage',
          data: encodeErrorResult({ abi: MessageQueueAbi, errorName: kind === 'other' ? 'InvalidMerkleProof' : 'MerkleRootDelayNotPassed' }),
        });
        return {};
      },
    } as unknown as PublicClient;
    try {
      const queue = new MessageQueueClient(fixture.params.messageQueueAddress, client,
        { prepareTransactionRequest: async () => ({}), sendRawTransaction: broadcast } as unknown as WalletClient,
        { address: fixture.params.sender, type: 'local', signTransaction: sign } as unknown as Account);
      const pending = queue.processMessage(12n, message, proof, { kind: 'application' }, undefined, Date.now() + 10,
        { wait: kind !== 'disabled' });
      await vi.advanceTimersByTimeAsync(11);
      const result = await pending;
      expect(result.success).toBe(kind === 'matures');
      expect(sign).toHaveBeenCalledTimes(kind === 'matures' ? 1 : 0);
      expect(broadcast).toHaveBeenCalledTimes(kind === 'matures' ? 1 : 0);
      if (kind === 'matures') expect(simulated).toEqual([getProcessMessageArgs(12n, message, proof), getProcessMessageArgs(12n, message, proof)]);
      if (kind === 'deadline') expect(result.error).toContain('deadline expired');
      if (kind === 'root' || kind === 'anchor') expect(simulated).toHaveLength(1);
    } finally { vi.useRealTimers(); }
  });

  test.each(['source-regression', 'source-history', 'ethereum-regression', 'ethereum-history'])('discovery retains previous finalized observations: %s', async (kind) => {
    vi.useFakeTimers();
    const fixture = rootsFixture();
    fixture.roots.clear();
    let polls = 0;
    fixture.params.gearApi.blocks.getFinalizedHead = async () => {
      polls++;
      return pin(hash(polls <= 2 ? 20 : polls === 3 ? 21 : kind === 'source-regression' ? 20 : 22)) as never;
    };
    fixture.params.gearApi.blocks.getBlockHash = async height =>
      pin(hash(kind === 'source-history' && polls >= 4 && BigInt(height.toString()) === 21n ? 99 : Number(height))) as never;
    fixture.params.ethereumPublicClient.getBlock = async ({ blockTag, blockNumber } = {}) => {
      const height = blockTag === 'finalized' ? (polls <= 2 ? 2n : polls === 3 ? 3n : kind === 'ethereum-regression' ? 2n : 4n) : blockNumber!;
      return { number: height, hash: hash(kind === 'ethereum-history' && polls >= 4 && height === 3n ? 99 : Number(height)) } as never;
    };
    try {
      const pending = prepareVaraToEthRelay({ ...fixture.params, wait: true }).catch(error => error);
      await vi.advanceTimersByTimeAsync(5);
      expect((await pending as Error).message).toContain(kind.startsWith('source') ? 'source history changed' : 'finalized queue history changed');
    } finally { vi.useRealTimers(); }
  });

  test('candidate proof is fetched at its resolved exact hash and rejected if that hash changes', async () => {
    const fixture = rootsFixture();
    fixture.roots.delete(8n);
    let candidateReads = 0;
    fixture.params.gearApi.blocks.getBlockHash = async height =>
      pin(hash(Number(height) === 12 && ++candidateReads > 1 ? 99 : Number(height))) as never;
    const proofRpc = vi.spyOn(fixture.params.gearApi.ethBridge, 'merkleProof');
    await expect(prepareVaraToEthRelay(fixture.params)).rejects.toThrow('Canonical finalized source or queue history changed');
    expect(proofRpc).toHaveBeenCalledWith(messageHash(message), hash(12));
  });

  test('a stalled simulation expires without signing or broadcasting even when it later resolves', async () => {
    vi.useFakeTimers();
    const fixture = receiptFixture();
    let finishSimulation!: () => void;
    const simulation = new Promise<void>((resolve) => { finishSimulation = resolve; });
    const prepare = vi.fn(async () => ({})), sign = vi.fn(async () => '0x01'), broadcast = vi.fn(async () => hash(7));
    try {
      const client = new MessageQueueClient(fixture.params.messageQueueAddress,
        { ...fixture.params.ethereumPublicClient, simulateContract: async () => simulation } as unknown as PublicClient,
        { prepareTransactionRequest: prepare, sendRawTransaction: broadcast, writeContract: broadcast } as unknown as WalletClient,
        { address: fixture.params.sender, type: 'local', signTransaction: sign } as unknown as Account);
      const pending = client.processMessage(12n, message, proof, { kind: 'application' }, undefined, Date.now() + 1000);
      await vi.advanceTimersByTimeAsync(1001);
      const result = await pending;
      expect(result.success).toBe(false);
      expect(result.transactionHash).toBe('0x');
      expect(result.error).toContain('deadline expired');
      finishSimulation();
      await vi.advanceTimersByTimeAsync(0);
      expect(prepare).not.toHaveBeenCalled();
      expect(sign).not.toHaveBeenCalled();
      expect(broadcast).not.toHaveBeenCalled();
    } finally { vi.useRealTimers(); }
  });
  test.each(['lost-response', 'wrong-hash'])('local broadcast %s retains the original signed hash without a replacement', async (fault) => {
    const fixture = receiptFixture();
    const signed = '0x01' as const;
    const sign = vi.fn(async () => signed);
    const broadcast = vi.fn(async () => {
      if (fault === 'lost-response') throw new Error('broadcast response lost');
      return hash(99);
    });
    const client = new MessageQueueClient(fixture.params.messageQueueAddress,
      { ...fixture.params.ethereumPublicClient, simulateContract: async () => ({}) } as unknown as PublicClient,
      { prepareTransactionRequest: async () => ({}), sendRawTransaction: broadcast } as unknown as WalletClient,
      { address: fixture.params.sender, type: 'local', signTransaction: sign } as unknown as Account);
    const result = await client.processMessage(12n, message, proof, { kind: 'application' });
    expect(result.success).toBe(false);
    expect(result.transactionHash).toBe(keccak256(signed));
    expect(result.error).toContain(fault === 'lost-response' ? 'broadcast response lost' : 'broadcast hash differs');
    expect(sign).toHaveBeenCalledTimes(1);
    expect(broadcast).toHaveBeenCalledTimes(1);
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
  let nonce: bigint, blockNumber: number, blockHash: HexString;
  beforeAll(async () => {
    expected = ['vara_to_eth_message_hash', 'vara_to_eth_root', 'vara_to_eth_proof', 'vara_to_eth_num_leaves', 'vara_to_eth_leaf_index', 'process_message_calldata']
      .map((name) => fs.readFileSync('test/tmp/' + name, 'utf8'));
    nonce = BigInt(process.env.VARA_TO_ETH_NONCE!);
    blockNumber = Number(process.env.VARA_TO_ETH_BLOCK_NUMBER!);
    gearApi = await GearApi.create({ providerAddress: process.env.VARA_WS_RPC });
    gearClient = new GearClient(gearApi);
    blockHash = (await gearApi.blocks.getBlockHash(blockNumber)).toHex();
  });
  afterAll(async () => { if (gearApi) await gearApi.disconnect(); });
  test('message hash', async () => {
    const msg = await gearClient.findMessageQueuedEvent(blockHash, nonce);
    if (!msg) throw new Error('Message not found');
    expect(messageHash(msg).slice(2)).toEqual(expected[0]);
  });
  test('merkle proof', async () => {
    const msg = await gearClient.findMessageQueuedEvent(blockHash, nonce);
    if (!msg) throw new Error('Message not found');
    const merkleProof = await gearClient.fetchMerkleProof(blockHash, messageHash(msg));
    expect(merkleProof.leafIndex.toString()).toEqual(expected[4]);
    expect(merkleProof.numLeaves.toString()).toEqual(expected[3]);
    expect(merkleProof.root.slice(2)).toEqual(expected[1]);
    expect(merkleProof.proof.map((item) => item.slice(2)).join('')).toEqual(expected[2]);
  });
  test('process message call', async () => {
    const msg = await gearClient.findMessageQueuedEvent(blockHash, nonce);
    if (!msg) throw new Error('Message not found');
    const merkleProof = await gearClient.fetchMerkleProof(blockHash, messageHash(msg));
    const data = encodeFunctionData({ abi: MessageQueueAbi, functionName: 'processMessage', args: getProcessMessageArgs(BigInt(blockNumber), msg, merkleProof) });
    expect(data.slice(2)).toEqual(expected[5]);
  });
});
