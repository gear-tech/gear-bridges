import type { GearApi, HexString, MessageQueuedData, GearCoreMessageUserUserMessage } from '@gear-js/api';
import { GearClient } from '@gear-js/bridge';

export type BridgeReply = { nonce: bigint; ethTokenId: `0x${string}` };

export function assertSuccessfulReceipt<T extends { status: string; transactionHash: string }>(
  receipt: T,
  originalHash: string,
): T {
  if (receipt.transactionHash.toLowerCase() !== originalHash.toLowerCase())
    throw new Error(
      `HOLD: original Ethereum transaction ${originalHash} was replaced by ${receipt.transactionHash}. Reconcile before retrying.`,
    );
  if (receipt.status !== 'success') throw new Error(`Ethereum transaction reverted: ${receipt.transactionHash}`);
  return receipt;
}

export function assertBridgeReply(value: unknown, expectedEthTokenId: `0x${string}`): BridgeReply {
  if (!value || typeof value !== 'object') throw new Error('Missing bridge application result');
  if ('err' in value) {
    const detail = value.err instanceof Error ? value.err.message : JSON.stringify(value.err);
    throw new Error(`RequestBridging failed: ${detail}`);
  }
  if (!('ok' in value) || Object.keys(value).length !== 1 || !Array.isArray(value.ok) || value.ok.length !== 2)
    throw new Error('Malformed bridge application result');

  const [rawNonce, ethTokenId] = value.ok as unknown[];
  if (
    (typeof rawNonce !== 'bigint' && typeof rawNonce !== 'number' && typeof rawNonce !== 'string') ||
    (typeof rawNonce === 'number' && !Number.isSafeInteger(rawNonce)) ||
    (typeof rawNonce === 'string' && !/^(?:[0-9]+|0x[0-9a-fA-F]+)$/.test(rawNonce))
  )
    throw new Error('Invalid bridge nonce');
  const nonce = BigInt(rawNonce);
  if (nonce < 0n || nonce >= 1n << 256n) throw new Error('Bridge nonce is outside uint256');
  if (
    typeof ethTokenId !== 'string' ||
    !/^0x[0-9a-fA-F]{40}$/.test(ethTokenId) ||
    ethTokenId.toLowerCase() !== expectedEthTokenId.toLowerCase()
  )
    throw new Error('Bridge reply token does not match the selected destination token');
  return { nonce, ethTokenId: ethTokenId as `0x${string}` };
}

export async function submitVaraBridgeRequest<T extends BridgeReply & { txHash: string; messageId?: string }>(
  request: () => Promise<T>,
  shouldPayBridgingFee: boolean,
  payFees: (args: { nonce: bigint }) => Promise<void>,
): Promise<T> {
  const original = await request();
  if (!shouldPayBridgingFee) return original;

  try {
    await payFees({ nonce: original.nonce });
  } catch (error) {
    throw new Error(
      `Source request succeeded: transaction ${original.txHash}, message ${original.messageId}, nonce ${original.nonce}. Recover fees from its history row; do not submit another bridge request. ${error instanceof Error ? error.message : String(error)}`,
    );
  }
  return original;
}

export async function waitForFinalizedReplies(
  api: GearApi,
  blockHash: HexString,
  queued: MessageQueuedData[],
  deadline: number,
): Promise<GearCoreMessageUserUserMessage[]> {
  const original = await api.rpc.chain.getHeader(blockHash);
  const replies = new Map<string, GearCoreMessageUserUserMessage>();
  const expected = new Map(queued.map((message) => [message.id.toHex(), message]));
  if (!queued.length || expected.size !== queued.length) throw new Error('Invalid original queued message identities');
  const gearClient = new GearClient(api);
  const messageIds = new Set(expected.keys());
  let next = original.number.toBigInt();
  let previousHash: string | undefined;
  let previousFinalized: { height: bigint; hash: string } | undefined;
  while (Date.now() < deadline) {
    const headHash = (await api.rpc.chain.getFinalizedHead()).toHex();
    const head = await api.rpc.chain.getHeader(headHash);
    const height = head.number.toBigInt();
    if (height < original.number.toBigInt() || (previousFinalized && height < previousFinalized.height))
      throw new Error('Finalized source height regressed');
    if (
      previousFinalized &&
      (await api.blocks.getBlockHash(previousFinalized.height)).toHex() !== previousFinalized.hash
    )
      throw new Error('Previous finalized source anchor changed');
    if ((await api.blocks.getBlockHash(original.number.toBigInt())).toHex() !== blockHash)
      throw new Error('Original source inclusion is no longer canonical');
    for await (const block of gearClient.finalizedReplyBlocks(next, height, messageIds, deadline, previousHash)) {
      const { number, hash } = block;
      next = number;
      for (const message of block.replies()) {
        const replyTo = message.details.unwrap().to.toHex();
        const request = expected.get(replyTo)!;
        if (replies.has(replyTo) || !message.source.eq(request.destination) || !message.destination.eq(request.source))
          throw new Error('Original reply identity is conflicting');
        replies.set(replyTo, message);
      }
      if ((await api.blocks.getBlockHash(next)).toHex() !== hash) throw new Error('Finalized reply pin changed');
      previousHash = hash;
      if (replies.size === queued.length) break;
      next++;
    }
    if ((await api.blocks.getBlockHash(height)).toHex() !== headHash) throw new Error('Finalized reply anchor changed');
    previousFinalized = { height, hash: headHash };
    if (Date.now() >= deadline) throw new Error('Original finalized reply deadline expired; reconcile before retrying');
    if (replies.size === queued.length) return queued.map((message) => replies.get(message.id.toHex())!);
    await new Promise((resolve) => setTimeout(resolve, Math.min(6000, Math.max(0, deadline - Date.now()))));
  }
  throw new Error('Original finalized reply deadline expired; reconcile before retrying');
}
