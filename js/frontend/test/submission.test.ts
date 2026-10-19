import assert from 'node:assert/strict';

import {
  assertBridgeReply,
  assertSuccessfulReceipt,
  submitVaraBridgeRequest,
  waitForFinalizedReplies,
} from '../src/features/swap/submission.ts';

const token = '0xabcdefabcdefabcdefabcdefabcdefabcdefabcd';
assert.deepEqual(assertBridgeReply({ ok: [0, token] }, token), { nonce: 0n, ethTokenId: token });
assert.equal(assertBridgeReply({ ok: ['0x10', token.toUpperCase().replace('0X', '0x')] }, token).nonce, 16n);
assert.equal(assertBridgeReply({ ok: [(1n << 256n) - 1n, token] }, token).nonce, (1n << 256n) - 1n);
assert.throws(() => assertBridgeReply({ err: { notEnoughGas: null } }, token), /notEnoughGas/);
for (const nonce of [-1, 1.1, Number.MAX_SAFE_INTEGER + 1, '', ' 1', '1e3', '0x', 1n << 256n, null])
  assert.throws(() => assertBridgeReply({ ok: [nonce, token] }, token), /nonce/);
for (const value of [null, {}, { ok: [0] }, { ok: [0, token], err: 'failed' }])
  assert.throws(() => assertBridgeReply(value, token));
assert.throws(() => assertBridgeReply({ ok: [0, '0x' + '12'.repeat(20)] }, token), /destination token/);
assert.throws(() => assertBridgeReply({ ok: [0, '0x12'] }, token), /destination token/);
const receipt = { status: 'success', transactionHash: '0xoriginal' };
assert.equal(assertSuccessfulReceipt(receipt, receipt.transactionHash), receipt);
assert.throws(
  () => assertSuccessfulReceipt({ ...receipt, status: 'reverted' }, receipt.transactionHash),
  /Ethereum transaction reverted: 0xoriginal/,
);
assert.throws(
  () => assertSuccessfulReceipt(receipt, '0xdifferent'),
  /original Ethereum transaction 0xdifferent was replaced/,
);
assert.equal(assertSuccessfulReceipt(receipt, receipt.transactionHash.toUpperCase()), receipt);

const hash = (height: bigint) => ('0x' + height.toString(16).padStart(64, '0')) as `0x${string}`;
const codec = (value: string) => ({
  toHex: () => value,
  eq: (other: { toHex: () => string }) => value === other.toHex(),
});
const request = { id: codec(hash(100n)), source: codec(hash(200n)), destination: codec(hash(300n)) };
const queued = [request] as unknown as Parameters<typeof waitForFinalizedReplies>[2];
function replyFixture({
  heads = [2n],
  badParent = false,
  badSource = false,
  duplicate = false,
  changedOriginal = false,
  changedPrevious = false,
}: {
  heads?: bigint[];
  badParent?: boolean;
  badSource?: boolean;
  duplicate?: boolean;
  changedOriginal?: boolean;
  changedPrevious?: boolean;
} = {}) {
  let poll = 0;
  const scanned: string[] = [];
  const message = {
    source: badSource ? codec(hash(999n)) : request.destination,
    destination: request.source,
    details: { isNone: false, unwrap: () => ({ to: request.id }) },
  };
  const event = { event: { data: [message] } };
  const api = {
    rpc: {
      chain: {
        getFinalizedHead: () => Promise.resolve(codec(hash(heads[Math.min(poll++, heads.length - 1)]))),
        getHeader: (value: string) =>
          Promise.resolve({
            number: { toBigInt: () => BigInt(value) },
            parentHash: codec(hash(badParent && BigInt(value) === 2n ? 0n : BigInt(value) - 1n)),
          }),
      },
    },
    blocks: {
      getBlockHash: (height: bigint) =>
        Promise.resolve(
          codec(
            hash((changedOriginal && height === 1n) || (changedPrevious && poll > 1 && height === 2n) ? 999n : height),
          ),
        ),
      getEvents: (value: string) => {
        scanned.push(value);
        return Promise.resolve(BigInt(value) === 2n ? (duplicate ? [event, event] : [event]) : []);
      },
    },
    events: { gear: { UserMessageSent: { is: () => true } } },
  } as unknown as Parameters<typeof waitForFinalizedReplies>[0];
  return { api, message, scanned };
}
const finalized = replyFixture();
assert.deepEqual(await waitForFinalizedReplies(finalized.api, hash(1n), queued, Date.now() + 1000), [
  finalized.message,
]);
assert.deepEqual(finalized.scanned, [hash(1n), hash(2n)]);
for (const options of [{ badParent: true }, { badSource: true }, { duplicate: true }, { changedOriginal: true }]) {
  const fixture = replyFixture(options);
  await assert.rejects(
    waitForFinalizedReplies(fixture.api, hash(1n), queued, Date.now() + 1000),
    /history changed|identity is conflicting|no longer canonical/,
  );
}
const delayed = replyFixture({ heads: [1n, 2n] });
const pendingReply = waitForFinalizedReplies(delayed.api, hash(1n), queued, Date.now() + 10000);
await new Promise((resolve) => setTimeout(resolve, 10));
assert.deepEqual(delayed.scanned, [hash(1n)], 'Hot reply must not be read before finalized coverage');
assert.deepEqual(await pendingReply, [delayed.message]);
for (const options of [{ heads: [1n, 0n] }, { heads: [2n, 3n], changedPrevious: true }]) {
  const fixture = replyFixture(options);
  // Keep the original pending so another finality observation is required.
  await assert.rejects(
    waitForFinalizedReplies(
      fixture.api,
      hash(1n),
      [{ ...request, id: codec(hash(101n)) }] as unknown as typeof queued,
      Date.now() + 10000,
    ),
    /regressed|Previous finalized source anchor changed/,
  );
}
await assert.rejects(waitForFinalizedReplies(finalized.api, hash(1n), queued, Date.now() - 1), /deadline expired/);

const originalBridge = {
  txHash: '0xoriginal-source',
  messageId: '0xoriginal-message',
  ...assertBridgeReply({ ok: [42n, token] }, token),
};
let sourceRequests = 0;
let finalizeSource!: () => void;
const sourceFinality = new Promise<void>((resolve) => {
  finalizeSource = resolve;
});
let finalizeFee!: () => void;
const feeFinality = new Promise<void>((resolve) => {
  finalizeFee = resolve;
});
let startFee!: () => void;
const feeStarted = new Promise<void>((resolve) => {
  startFee = resolve;
});
const paidNonces: bigint[] = [];
const payFees = async ({ nonce }: { nonce: bigint }) => {
  paidNonces.push(nonce);
  startFee();
  await feeFinality;
};
const submitOriginal = async () => {
  sourceRequests++;
  await sourceFinality;
  return originalBridge;
};
let completed = false;
const automatic = submitVaraBridgeRequest(submitOriginal, true, payFees).then((result) => {
  completed = true;
  return result;
});
assert.equal(sourceRequests, 1);
assert.deepEqual(paidNonces, [], 'Fee payment must wait for the finalized original nonce');
finalizeSource();
await feeStarted;
assert.deepEqual(paidNonces, [42n]);
assert.equal(completed, false, 'Automatic submission must wait for finalized fee payment');
finalizeFee();
assert.equal(await automatic, originalBridge);
assert.equal(sourceRequests, 1);

let failFee = true;
const retryableFee = ({ nonce }: { nonce: bigint }) => {
  assert.equal(nonce, originalBridge.nonce);
  return failFee ? Promise.reject(new Error('Fee rejected')) : Promise.resolve();
};
await assert.rejects(submitVaraBridgeRequest(submitOriginal, true, retryableFee), (error: unknown) => {
  assert.ok(error instanceof Error);
  for (const evidence of [originalBridge.txHash, originalBridge.messageId, 'nonce 42', 'history row', 'Fee rejected'])
    assert.ok(error.message.includes(evidence));
  assert.ok(error.message.includes('do not submit another bridge request'));
  return true;
});
assert.equal(sourceRequests, 2, 'Fee failure must not resubmit the source request');
failFee = false;
await retryableFee({ nonce: originalBridge.nonce });
assert.equal(sourceRequests, 2, 'History fee retry must only pay the original nonce');
assert.equal(
  await submitVaraBridgeRequest(submitOriginal, false, () => {
    assert.fail('Manual claim must not pay fees');
  }),
  originalBridge,
);
assert.equal(sourceRequests, 3);
for (const failedRequest of [
  () => Promise.reject(new Error('Original request failed')),
  () =>
    Promise.resolve().then(() => ({ ...originalBridge, ...assertBridgeReply({ err: 'Bridge reply failed' }, token) })),
]) {
  await assert.rejects(
    submitVaraBridgeRequest(failedRequest, true, () => {
      assert.fail('Failed source request must not pay fees');
    }),
    /Original request failed|Bridge reply failed/,
  );
}
console.log(
  'Submission assertions passed: finalized original nonce then ordinary fees, manual claims, fee recovery, original hashes and canonical replies',
);
