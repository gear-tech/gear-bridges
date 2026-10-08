import { Account, PublicClient, WalletClient } from 'viem';
import { GearApi, HexString } from '@gear-js/api';
import { hexToBn } from '@polkadot/util';

import { getMessageQueueClient, getProcessMessageArgs } from '../ethereum/index.js';
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
  const originalHash = await gearApi.blocks.getBlockHash(blockNumber);
  const message = await gearClient.findMessageQueuedEvent(Number(blockNumber), nonce);
  if (!message) throw new Error('Message with nonce ' + nonce + ' is unavailable in original finalized block ' + blockNumber);
  const hash = messageHash(message);
  let scanFrom = 0n;
  const candidates = new Map<bigint, HexString>();
  const failures = new Map<bigint, string>();
  for (;;) {
    if (Date.now() >= deadline) throw new Error('Original relay deadline expired during stored-root discovery');
    const sourceFinalizedHash = await gearApi.blocks.getFinalizedHead();
    const sourceFinalized = await gearApi.rpc.chain.getHeader(sourceFinalizedHash);
    if (sourceFinalized.number.toBigInt() < originalSourceFinalized.number.toBigInt() ||
        !(await gearApi.blocks.getBlockHash(originalSourceFinalized.number.toBigInt())).eq(originalSourceFinalizedHash)) {
      throw new Error('Original canonical finalized source history changed');
    }
    const finalized = await ethereumPublicClient.getBlock({ blockTag: 'finalized' });
    const tryRoot = async (height: bigint, root: HexString): Promise<PreparedVaraToEthRelay | null> => {
      if (height < blockNumber || height > sourceFinalized.number.toBigInt()) return null;
      if (await queue.getMerkleRoot(height, finalized.number) !== root) return null;
      let proof: Proof;
      try {
        proof = await gearClient.fetchMerkleProof(Number(height), hash);
      } catch (error) {
        failures.set(height, error instanceof Error ? error.message : 'Historical proof unavailable');
        return null;
      }
      if (proof.root.toLowerCase() !== root.toLowerCase() || proof.numLeaves <= 0n ||
          proof.leafIndex < 0n || proof.leafIndex >= proof.numLeaves) {
        failures.set(height, 'Historical inclusion proof does not match the selected stored root');
        return null;
      }
      if (!(await gearApi.blocks.getBlockHash(blockNumber)).eq(originalHash) ||
          !(await gearApi.blocks.getBlockHash(sourceFinalized.number.toBigInt())).eq(sourceFinalizedHash) ||
          (await ethereumPublicClient.getBlock({ blockNumber: finalized.number })).hash !== finalized.hash) {
        throw new Error('Canonical finalized source or queue history changed during proof preparation');
      }
      statusCb('Historical message proof matched stored root', { blockNumber: height.toString(), merkleRoot: root, msgHash: hash });
      return { blockNumber: height, message, proof, args: getProcessMessageArgs(height, message, proof) };
    };
    const exact = await queue.getMerkleRoot(blockNumber, finalized.number);
    if (exact) {
      const prepared = await tryRoot(blockNumber, exact);
      if (prepared) return prepared;
    }
    for (; scanFrom <= finalized.number; scanFrom += 2000n) {
      const end = scanFrom + 1999n < finalized.number ? scanFrom + 1999n : finalized.number;
      const logs = await queue.getMerkleRootLogsInRange(scanFrom, end);
      for (const log of logs) {
        if (!log.removed && log.args.blockNumber >= blockNumber) candidates.set(log.args.blockNumber, log.args.merkleRoot);
      }
      if (end === finalized.number) {
        scanFrom = end + 1n;
        break;
      }
    }
    const heights = [...candidates.keys()].sort((a, b) => a < b ? -1 : a > b ? 1 : 0);
    for (const height of heights) {
      if (height === blockNumber && exact) continue;
      const prepared = await tryRoot(height, candidates.get(height)!);
      if (prepared) return prepared;
    }
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
  return queue.processMessage(prepared.blockNumber, prepared.message, prepared.proof, params.expectedEffect, params.statusCb, deadline);
}
