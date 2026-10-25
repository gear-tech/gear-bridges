import assert from 'node:assert/strict';
import { mock } from 'node:test';

import type { ISubmittableResult } from '@polkadot/types/types';

import {
  assertBridgeReply,
  assertSuccessfulReceipt,
  signAndSendFinalized,
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

// Exercise the same subscription lifecycle used by the hook without a React renderer.
mock.timers.enable({ apis: ['Date', 'setTimeout'] });
try {
  const timeout = 44 * 60 * 1000;
  type Result = ISubmittableResult;
  const result = (type: string) =>
    ({
      txHash: codec('0xsigned-original'),
      status: {
        type,
        isFinalized: type === 'Finalized',
        isDropped: type === 'Dropped',
        isInvalid: type === 'Invalid',
        isUsurped: type === 'Usurped',
        isFinalityTimeout: type === 'FinalityTimeout',
        asFinalized: codec(hash(1n)),
      },
      events: [],
    }) as unknown as Result;
  function submissionFixture() {
    let callback!: (value: Result) => void;
    let admit!: (value: () => void) => void;
    let rejectSigning!: (reason: Error) => void;
    const wallet = new Promise<() => void>((resolve, reject) => {
      admit = resolve;
      rejectSigning = reject;
    });
    const unsubscribe = mock.fn();
    const extrinsic = {
      isSigned: false,
      signedHash: '0xsigned-original',
      get hash() {
        return codec(this.isSigned ? this.signedHash : '0xunsigned');
      },
      signAndSend: mock.fn((_address: unknown, _options: unknown, cb: (value: Result) => void) => {
        callback = cb;
        return wallet;
      }),
    };
    const start = (deadline = Date.now() + timeout) =>
      signAndSendFinalized(extrinsic as unknown as Parameters<typeof signAndSendFinalized>[0], 'account', {}, deadline);
    const emit = (type: string) => {
      extrinsic.isSigned = true;
      const value = result(type);
      callback(value);
      return value;
    };
    return { start, emit, extrinsic, admit: () => admit(unsubscribe), rejectSigning, unsubscribe };
  }
  const stalled = submissionFixture();
  let settled = false;
  const stalledPromise = stalled.start().finally(() => {
    settled = true;
  });
  const stalledRejection = assert.rejects(
    stalledPromise,
    /HOLD:.*deadline expired; transaction outcome is unknown.*Original transaction 0xsigned-original.*Do not submit another transaction; reconcile/,
  );
  stalled.emit('InBlock');
  stalled.extrinsic.signedHash = '0xchanged-extrinsic';
  mock.timers.tick(timeout - 1);
  await Promise.resolve();
  assert.equal(settled, false, 'InBlock alone cannot settle the mutation');
  mock.timers.tick(1);
  await stalledRejection;
  stalled.emit('Finalized');
  stalled.admit();
  await Promise.resolve();
  assert.equal(stalled.unsubscribe.mock.callCount(), 1, 'Late unsubscribe must still be called exactly once');
  assert.equal(stalled.extrinsic.signAndSend.mock.callCount(), 1, 'An unknown outcome must not resubmit');

  const normal = submissionFixture();
  const normalPromise = normal.start();
  const finalizedResult = normal.emit('Finalized');
  assert.equal(await normalPromise, finalizedResult, 'Normal finality must preserve the original result');
  normal.emit('Invalid');
  normal.admit();
  await Promise.resolve();
  assert.equal(normal.unsubscribe.mock.callCount(), 1);

  const admitted = submissionFixture();
  const admittedPromise = admitted.start();
  const admittedRejection = assert.rejects(admittedPromise, /Original transaction 0xsigned-original/);
  admitted.extrinsic.isSigned = true;
  admitted.admit();
  await Promise.resolve();
  mock.timers.tick(timeout);
  await admittedRejection;
  assert.equal(admitted.unsubscribe.mock.callCount(), 1, 'Expiry must unsubscribe an admitted silent submission');

  const unsigned = submissionFixture();
  const unsignedPromise = unsigned.start();
  const unsignedRejection = assert.rejects(unsignedPromise, (error: unknown) => {
    assert.ok(error instanceof Error);
    assert.match(error.message, /Signed transaction hash unavailable; the pending wallet request may still complete/);
    assert.match(error.message, /Do not submit another transaction; reconcile/);
    assert.ok(!error.message.includes('0xunsigned'), 'Unsigned bytes must not be reported as an original tx hash');
    return true;
  });
  mock.timers.tick(timeout);
  await unsignedRejection;
  unsigned.emit('Finalized');
  unsigned.admit();
  await Promise.resolve();
  assert.equal(unsigned.unsubscribe.mock.callCount(), 1, 'Signing after expiry must only clean up, not succeed');
  assert.equal(unsigned.extrinsic.signAndSend.mock.callCount(), 1, 'A silent wallet must not be resubmitted');

  const racing = submissionFixture();
  const racingPromise = racing.start();
  const racingRejection = assert.rejects(racingPromise, /deadline expired.*Original transaction 0xsigned-original/);
  // Advance wall time without running the timer: a queued callback must enforce the deadline too.
  mock.timers.setTime(Date.now() + timeout);
  racing.emit('Finalized');
  await racingRejection;
  racing.admit();
  await Promise.resolve();
  assert.equal(racing.unsubscribe.mock.callCount(), 1);

  for (const status of ['Dropped', 'Invalid', 'Usurped', 'FinalityTimeout']) {
    const terminal = submissionFixture();
    const pending = terminal.start();
    const rejection = assert.rejects(pending, new RegExp(`0xsigned-original: ${status}`));
    terminal.admit();
    await Promise.resolve();
    terminal.emit(status);
    await rejection;
    assert.equal(terminal.unsubscribe.mock.callCount(), 1);
    terminal.emit('Finalized');
  }
  for (const synchronous of [false, true]) {
    const rejected = submissionFixture();
    if (synchronous)
      rejected.extrinsic.signAndSend.mock.mockImplementation(() => {
        throw new Error('Wallet rejected');
      });
    const pending = rejected.start();
    const rejection = assert.rejects(pending, /Wallet rejected/);
    if (!synchronous) rejected.rejectSigning(new Error('Wallet rejected'));
    await rejection;
    assert.equal(rejected.unsubscribe.mock.callCount(), 0);
  }
  const expired = submissionFixture();
  await assert.rejects(expired.start(Date.now()), /deadline expired/);
  assert.equal(expired.extrinsic.signAndSend.mock.callCount(), 0, 'An expired request must not enter the wallet');
} finally {
  mock.restoreAll();
  mock.timers.reset();
}
console.log(
  'Submission assertions passed: finalized inclusion deadline and cleanup, original nonce then fees, fee recovery, original hashes and canonical replies',
);
