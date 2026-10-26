import { Account, PublicClient, WalletClient, BaseError, ContractFunctionRevertedError } from 'viem';
import { GearApi, HexString } from '@gear-js/api';
import { hexToBn } from '@polkadot/util';
import type { Bytes } from '@polkadot/types';

import { getMessageQueueClient, getProcessMessageArgs, MessageQueueAbi } from '../ethereum/index.js';
import { GearClient } from '../vara/index.js';
import { VaraMessage, Proof } from '../vara/types.js';
import { messageHash } from './util.js';
import { StatusCb, withOriginalDeadline } from '../util.js';
import type { OutboundEffect } from '../ethereum/message-queue.js';

/**
 * Parameters for relaying a Vara network message to Ethereum.
 * This interface defines all the required configuration and optional settings
 * needed to relay cross-chain messages from Vara to Ethereum.
 */
export type RelayVaraToEthParams = {
  /**
   * The message nonce to relay (bigint or hex string, little endian encoded if hex)
   */
  nonce: bigint | HexString;
  /**
   * The Vara block number containing the initial transaction
   */
  blockNumber: bigint;
  /**
   * Viem public client for reading Ethereum blockchain state
   */
  ethereumPublicClient: PublicClient;
  /**
   * Viem wallet client for sending Ethereum transactions
   */
  ethereumWalletClient: WalletClient;
  /**
   * Ethereum account to use for transaction signing and sending
   */
  ethereumAccount: Account;
  /**
   * Gear API instance for interacting with the Vara network
   */
  gearApi: GearApi;
  /**
   * Address of the Ethereum message queue contract
   */
  messageQueueAddress: `0x${string}`;
  expectedEffect: OutboundEffect;
  /**
   * If true, waits for MerkleRoot to appear on MessageQueue contract instead of throwing error
   */
  wait?: boolean;
  /**
   * Optional callback function to track relay operation status
   */
  statusCb?: StatusCb;
  deadline?: number;
  /** Original queue creation block; verified on chain and used only as a discovery hint. */
  messageQueueDeploymentBlock?: bigint;
};

export type PrepareVaraToEthParams = Omit<RelayVaraToEthParams, 'ethereumWalletClient' | 'ethereumAccount' | 'expectedEffect'>;

export type PreparedVaraToEthRelay = {
  blockNumber: bigint;
  message: VaraMessage;
  proof: Proof;
  args: ReturnType<typeof getProcessMessageArgs>;
};

/** Discover a proof against the actual stored root, regardless of GRANDPA handovers. */
export function prepareVaraToEthRelay(params: PrepareVaraToEthParams): Promise<PreparedVaraToEthRelay> {
  const deadline = params.deadline ?? Date.now() + 44 * 60 * 1000;
  return withOriginalDeadline(deadline, () => prepareOriginalVaraToEthRelay({ ...params, deadline }));
}

async function prepareOriginalVaraToEthRelay(params: PrepareVaraToEthParams & { deadline: number }): Promise<PreparedVaraToEthRelay> {
  const { gearApi, ethereumPublicClient, messageQueueAddress, blockNumber, wait = false, statusCb = () => {} } = params;
  const deadline = params.deadline;
  const checkDeadline = () => {
    if (Date.now() >= deadline) throw new Error('Original relay deadline expired during stored-root discovery');
  };
  const nonce = typeof params.nonce === 'string'
    ? (() => {
        if (!/^0x[0-9a-fA-F]{64}$/.test(params.nonce)) throw new Error('Hex nonce must contain exactly 32 little-endian bytes');
        return BigInt(hexToBn(params.nonce, { isLe: true }).toString());
      })()
    : params.nonce;
  if (nonce < 0n || nonce >= 1n << 256n) throw new Error('Nonce is outside the uint256 range');
  if (blockNumber < 0n || blockNumber > 0xffffffffn) throw new Error('Source block number is outside the uint32 range');
  const originalSourceFinalizedHash = await gearApi.blocks.getFinalizedHead();
  const originalSourceFinalized = await gearApi.rpc.chain.getHeader(originalSourceFinalizedHash);
  if (blockNumber > originalSourceFinalized.number.toBigInt()) throw new Error('Original queued message block is not finalized');
  const gearClient = new GearClient(gearApi);
  const queue = getMessageQueueClient(messageQueueAddress, ethereumPublicClient);
  const originalHash = (await gearApi.blocks.getBlockHash(blockNumber)).toHex();
  if ((await gearApi.rpc.chain.getHeader(originalHash)).number.toBigInt() !== blockNumber) {
    throw new Error('Original queued-message header height mismatch');
  }
  const message = await gearClient.findMessageQueuedEvent(originalHash, nonce);
  if (!message) throw new Error('Message with nonce ' + nonce + ' is unavailable in original finalized block ' + blockNumber);
  const hash = messageHash(message);
  // Read raw storage at historical metadata/hash: an absent ValueQuery is not
  // evidence for QueueId zero. Unknown runtime/storage remains a release gate.
  const queueIds = new Map<HexString, bigint>();
  const queueIdAt = async (at: HexString): Promise<bigint> => {
    const cached = queueIds.get(at);
    if (cached !== undefined) return cached;
    const query = (await gearApi.at(at)).query.gearEthBridge?.queueId;
    if (!query) throw new Error('HOLD: historical QueueId metadata unavailable at ' + at);
    const raw = await gearApi.rpc.state.getStorage<Bytes>(query.key(), at);
    checkDeadline();
    if (!raw || raw.isEmpty || raw.toU8a(true).length !== 8) {
      throw new Error('HOLD: historical QueueId evidence unavailable or malformed at ' + at);
    }
    const id = BigInt(hexToBn(raw.toHex(), { isLe: true }).toString());
    queueIds.set(at, id);
    return id;
  };
  const originalQueueId = await queueIdAt(originalHash);
  let scanFrom = 0n;
  let discoveryFloor = 0n;
  let deploymentChecked = false;
  const candidates = new Map<bigint, HexString>();
  const failures = new Map<bigint, string>();
  let previousSource = { height: originalSourceFinalized.number.toBigInt(), hash: originalSourceFinalizedHash.toHex() };
  let originalEthereum: { height: bigint; hash: HexString } | undefined;
  let previousEthereum: { height: bigint; hash: HexString } | undefined;
  for (;;) {
    checkDeadline();
    const sourceFinalizedHash = await gearApi.blocks.getFinalizedHead();
    const sourceFinalized = await gearApi.rpc.chain.getHeader(sourceFinalizedHash);
    if (sourceFinalized.number.toBigInt() < previousSource.height ||
        !(await gearApi.blocks.getBlockHash(previousSource.height)).eq(previousSource.hash) ||
        !(await gearApi.blocks.getBlockHash(originalSourceFinalized.number.toBigInt())).eq(originalSourceFinalizedHash)) {
      throw new Error('Original canonical finalized source history changed');
    }
    const priorSource = previousSource;
    previousSource = { height: sourceFinalized.number.toBigInt(), hash: sourceFinalizedHash.toHex() };
    const finalized = await ethereumPublicClient.getBlock({ blockTag: 'finalized' });
    if (typeof finalized.number !== 'bigint' || finalized.number < 0n || !/^0x[0-9a-f]{64}$/i.test(finalized.hash ?? '')) {
      throw new Error('Malformed finalized queue anchor');
    }
    if (previousEthereum && (finalized.number < previousEthereum.height ||
        (await ethereumPublicClient.getBlock({ blockNumber: previousEthereum.height })).hash !== previousEthereum.hash)) {
      throw new Error('Previously observed finalized queue history changed');
    }
    if (originalEthereum && (await ethereumPublicClient.getBlock({ blockNumber: originalEthereum.height })).hash !== originalEthereum.hash) {
      throw new Error('Original finalized queue history changed');
    }
    originalEthereum ??= { height: finalized.number, hash: finalized.hash };
    const priorEthereum = previousEthereum;
    previousEthereum = { height: finalized.number, hash: finalized.hash };
    const tryRoot = async (height: bigint, root: HexString): Promise<PreparedVaraToEthRelay | null> => {
      checkDeadline();
      if (height < blockNumber || height > sourceFinalized.number.toBigInt()) return null;
      const storedRoot = await queue.getMerkleRoot(height, finalized.number);
      checkDeadline();
      if (storedRoot !== root) return null;
      const candidateHash = (await gearApi.blocks.getBlockHash(height)).toHex();
      if ((await gearApi.rpc.chain.getHeader(candidateHash)).number.toBigInt() !== height) {
        throw new Error('Candidate proof header height mismatch');
      }
      let proof: Proof;
      try {
        if (await queueIdAt(candidateHash) !== originalQueueId) {
          failures.set(height, 'Stored root belongs to a different authenticated QueueId');
          return null;
        }
        proof = await gearClient.fetchMerkleProof(candidateHash, hash);
      } catch (error) {
        failures.set(height, error instanceof Error ? error.message : 'Historical proof unavailable');
        checkDeadline();
        return null;
      }
      checkDeadline();
      if (proof.root.toLowerCase() !== root.toLowerCase() || proof.numLeaves <= 0n ||
          proof.leafIndex < 0n || proof.leafIndex >= proof.numLeaves) {
        failures.set(height, 'Historical inclusion proof does not match the selected stored root');
        return null;
      }
      const args = getProcessMessageArgs(height, message, proof);
      try {
        // Execute the contract's selected-block maturity policy at the retained
        // finalized pin, including its governance-specific delay. Never borrow
        // the timestamp of the original message or an earlier cumulative root.
        await ethereumPublicClient.simulateContract({
          address: messageQueueAddress, abi: MessageQueueAbi, functionName: 'processMessage',
          args, blockNumber: finalized.number,
        });
      } catch (error) {
        const revert = error instanceof BaseError ? error.walk(cause => cause instanceof ContractFunctionRevertedError) : undefined;
        const reason = revert instanceof ContractFunctionRevertedError ? revert.data?.errorName : undefined;
        if (!reason || !['MerkleRootDelayNotPassed', 'InvalidMerkleProof'].includes(reason)) throw error;
        failures.set(height, reason);
        checkDeadline();
        return null;
      }
      if (!(await gearApi.blocks.getBlockHash(priorSource.height)).eq(priorSource.hash) ||
          (priorEthereum && (await ethereumPublicClient.getBlock({ blockNumber: priorEthereum.height })).hash !== priorEthereum.hash) ||
          !(await gearApi.blocks.getBlockHash(height)).eq(candidateHash) ||
          !(await gearApi.blocks.getBlockHash(blockNumber)).eq(originalHash) ||
          !(await gearApi.blocks.getBlockHash(sourceFinalized.number.toBigInt())).eq(sourceFinalizedHash) ||
          (await ethereumPublicClient.getBlock({ blockNumber: finalized.number })).hash !== finalized.hash) {
        throw new Error('Canonical finalized source or queue history changed during proof preparation');
      }
      checkDeadline();
      statusCb('Historical message proof matched stored root', { blockNumber: height.toString(), merkleRoot: root, msgHash: hash });
      return { blockNumber: height, message, proof, args };
    };
    const exact = await queue.getMerkleRoot(blockNumber, finalized.number);
    if (exact) {
      const prepared = await tryRoot(blockNumber, exact);
      if (prepared) return prepared;
    }
    if (!deploymentChecked) {
      deploymentChecked = true;
      const hint = params.messageQueueDeploymentBlock;
      if (hint !== undefined && hint > 0n && hint <= finalized.number) {
        // Deployment evidence only prioritizes discovery; unavailable or misleading hints never hide older roots.
        try {
          const before = await ethereumPublicClient.getBlock({ blockNumber: hint - 1n });
          const at = await ethereumPublicClient.getBlock({ blockNumber: hint });
          checkDeadline();
          const [priorCode, deployedCode] = await Promise.all([
            ethereumPublicClient.getBytecode({ address: messageQueueAddress, blockNumber: hint - 1n }),
            ethereumPublicClient.getBytecode({ address: messageQueueAddress, blockNumber: hint }),
          ]);
          checkDeadline();
          if (before.number === hint - 1n && at.number === hint && /^0x[0-9a-f]{64}$/i.test(before.hash ?? '') &&
              /^0x[0-9a-f]{64}$/i.test(at.hash ?? '') && (!priorCode || priorCode === '0x') && deployedCode && deployedCode !== '0x' &&
              (await ethereumPublicClient.getBlock({ blockNumber: hint - 1n })).hash === before.hash &&
              (await ethereumPublicClient.getBlock({ blockNumber: hint })).hash === at.hash) discoveryFloor = hint;
        } catch { checkDeadline(); }
      }
    }
    const tryCandidates = async (roots = candidates) => {
      const heights = [...roots.keys()].sort((a, b) => a < b ? -1 : a > b ? 1 : 0);
      for (const height of heights) {
        checkDeadline();
        if (height === blockNumber && exact) continue;
        const prepared = await tryRoot(height, roots.get(height)!);
        if (prepared) return prepared;
      }
      return null;
    };
    const cached = await tryCandidates();
    if (cached) return cached;
    const floor = scanFrom > discoveryFloor ? scanFrom : discoveryFloor;
    const ranges: [bigint, bigint][] = [[floor, finalized.number]];
    if (scanFrom < discoveryFloor) ranges.push([scanFrom, discoveryFloor - 1n]);
    for (const [start, finish] of ranges) {
      // Recent-first pages can return an authenticated claim immediately, without a genesis sweep.
      for (let end = finish; end >= start;) {
        checkDeadline();
        const from = end - 1999n > start ? end - 1999n : start;
        const logs = await queue.getMerkleRootLogsInRange(from, end);
        checkDeadline();
        const discovered = new Map<bigint, HexString>();
        for (const log of logs) {
          if (!log.removed && log.args.blockNumber >= blockNumber) {
            candidates.set(log.args.blockNumber, log.args.merkleRoot);
            discovered.set(log.args.blockNumber, log.args.merkleRoot);
          }
        }
        const prepared = await tryCandidates(discovered);
        if (prepared) return prepared;
        end = from - 1n;
      }
    }
    scanFrom = finalized.number + 1n;
    if (!wait) {
      throw new Error('No authenticated stored-root claim is available for original nonce ' + nonce +
        (failures.size ? ': ' + [...failures.entries()].map(([height, reason]) => height + ': ' + reason).join('; ') : ''));
    }
    statusCb('Waiting for a finalized stored root with an available original-message inclusion proof');
    await new Promise((resolve) => setTimeout(resolve, Math.max(1, Math.min(ethereumPublicClient.pollingInterval, deadline - Date.now()))));
  }
}

export async function relayVaraToEth(params: RelayVaraToEthParams) {
  const deadline = params.deadline ?? Date.now() + 44 * 60 * 1000;
  const prepared = await prepareVaraToEthRelay({ ...params, deadline });
  const queue = getMessageQueueClient(params.messageQueueAddress, params.ethereumPublicClient,
    params.ethereumWalletClient, params.ethereumAccount);
  return queue.processMessage(prepared.blockNumber, prepared.message, prepared.proof, params.expectedEffect, params.statusCb, deadline, { wait: params.wait ?? false });
}
