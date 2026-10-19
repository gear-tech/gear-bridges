import { decode as decodeRlp, encode as encodeRlp } from '@ethereumjs/rlp';
import { bytesToHex, concatHex, decodeEventLog, keccak256, stringToHex } from 'viem';
import type { HexString } from '@gear-js/api';

export interface InboundTokenEffect {
  readonly managerAddress: HexString;
  readonly sourceToken: HexString;
  readonly destinationToken: HexString;
  readonly sender: HexString;
  readonly receiver: HexString;
  readonly amount: bigint;
}

export interface SettledReceiptDeposit {
  log_index: number | string | bigint;
  sender: HexString;
  receiver: HexString;
  eth_token_id: HexString;
  token_id: HexString;
  amount: number | string | bigint;
  outcome: string;
}

export const VFT_MANAGER_IDL_SHA256 = '14f42d4ede5182f38e3fd5059bff97a877c02877c3892a9494152f7c48fcc698';
export const NATIVE_WRAPPER_IDL_SHA256 = 'afd6509bb15544b1302dcd4c5659bd77ef07c7dfb98a292f0c4a99b59c73fca9';

export interface NativeReceiptDeposit extends SettledReceiptDeposit {
  supply: 'Gear' | 'Ethereum'; native: boolean; operation_id: HexString; child: HexString | null;
}
export interface NativeRedemption {
  from: HexString; to: HexString; amount: number | string | bigint; child: HexString;
  status: 'Queued' | 'Delivered' | 'Returned' | 'Ambiguous' | 'Refunded'; returned_value: number | string | bigint;
}
export interface NativeTokenSettlement {
  operationId: HexString; logIndex: bigint; child: HexString;
  redemption: NativeRedemption & { status: 'Delivered' | 'Refunded' };
}
export interface NativeReceiptIdentity {
  managerId: HexString; proxyId: HexString; wrapperId: HexString; slot: bigint; transactionIndex: bigint;
  receiptRlp: Uint8Array; expectedEffect: InboundTokenEffect;
}
function u64Le(value: bigint): HexString {
  if (value < 0n || value > 0xffffffffffffffffn) throw new Error('Invalid native operation coordinate');
  const bytes = new Uint8Array(8); new DataView(bytes.buffer).setBigUint64(0, value, true);
  return bytesToHex(bytes);
}
/** Byte-identical to the actor's original fixed-array SCALE tuple; no Vec prefix. */
export function nativeOperationId(identity: NativeReceiptIdentity, logIndex: bigint): HexString {
  return keccak256(concatHex([stringToHex('vara/native-escrow/v1'), identity.managerId, identity.proxyId,
    identity.expectedEffect.managerAddress, u64Le(identity.slot), u64Le(identity.transactionIndex),
    u64Le(logIndex), keccak256(identity.receiptRlp)]));
}
const messageId = (id: HexString | null) => typeof id === 'string' && /^0x[0-9a-fA-F]{64}$/.test(id) && BigInt(id) !== 0n;

/** Validate the original known-pending cohort without treating its projected rows as settled. */
export function validateNativePendingCohort(identity: NativeReceiptIdentity, rows: readonly NativeReceiptDeposit[]): void {
  validateInboundTokenEffect(identity.receiptRlp, rows.map(row => ({ ...row, outcome: 'Settled' })), identity.expectedEffect);
  let native = 0;
  for (const row of rows) {
    if (row.outcome !== 'Settled' && !(row.native && (row.outcome === 'NativeQueued' || row.outcome === 'Refunded'))) throw new Error('HOLD: incomplete original native cohort');
    if (row.operation_id !== nativeOperationId(identity, BigInt(row.log_index))) throw new Error('HOLD: substituted original operation ID');
    if (row.native) {
      if (row.supply !== 'Gear' || row.token_id !== identity.wrapperId || !messageId(row.child)) throw new Error('HOLD: wrong native wrapper or original economic child');
      native++;
    }
  }
  if (!native) throw new Error('HOLD: native-pending reply lacks its original native operation');
}

/** Only original NativeQueued -> Settled/Refunded is allowed; terminal outcomes and economic identities remain fixed. */
export function validateNativeCohortContinuity(original: readonly NativeReceiptDeposit[], current: readonly NativeReceiptDeposit[], settled: boolean): void {
  if (current.length !== original.length) throw new Error('HOLD: native cohort row count changed');
  original.forEach((before, index) => {
    const after = current[index];
    if (BigInt(before.log_index) !== BigInt(after.log_index) || before.sender !== after.sender || before.receiver !== after.receiver ||
        before.eth_token_id !== after.eth_token_id || before.token_id !== after.token_id || BigInt(before.amount) !== BigInt(after.amount) ||
        before.supply !== after.supply || before.native !== after.native || before.operation_id !== after.operation_id || before.child !== after.child ||
        (before.outcome !== 'NativeQueued' ? after.outcome !== before.outcome :
          after.outcome !== 'Settled' && after.outcome !== 'Refunded' && !(after.outcome === 'NativeQueued' && !settled))) {
      throw new Error('HOLD: original native cohort identity or outcome changed');
    }
  });
}

/** Manager->wrapper child and wrapper->recipient payout child are different immutable identities. */
export function validateNativeRedemption(row: NativeReceiptDeposit, managerId: HexString, value: NativeRedemption | null, original?: NativeRedemption): 'Queued' | 'Delivered' | 'Refunded' {
  if (!value || !row.native || value.from !== managerId || value.to !== row.receiver || BigInt(value.amount) !== BigInt(row.amount) ||
      !messageId(value.child) || value.child === row.child ||
      (value.status !== 'Queued' && value.status !== 'Delivered' && value.status !== 'Refunded') ||
      BigInt(value.returned_value) !== (value.status === 'Refunded' ? BigInt(row.amount) : 0n) ||
      (original && (value.from !== original.from || value.to !== original.to || BigInt(value.amount) !== BigInt(original.amount) ||
        value.child !== original.child || (original.status !== 'Queued' &&
          (value.status !== original.status || BigInt(value.returned_value) !== BigInt(original.returned_value))))) ||
      (row.outcome === 'Settled' ? value.status !== 'Delivered' : row.outcome === 'Refunded' ? value.status !== 'Refunded' : row.outcome !== 'NativeQueued')) {
    throw new Error('HOLD: native payout missing, partially returned, ambiguous or substituted');
  }
  return value.status;
}

/** Refunded rows qualify only with the exact wrapper redemption, never through generic settled-token validation. */
export function validateNativeTokenEffect(identity: NativeReceiptIdentity, rows: readonly NativeReceiptDeposit[],
  redemptions: ReadonlyMap<HexString, NativeRedemption | null>, originals?: ReadonlyMap<HexString, NativeRedemption>): NativeTokenSettlement[] {
  validateNativePendingCohort(identity, rows);
  return rows.filter(row => row.native).map(row => {
    const redemption = redemptions.get(row.operation_id) ?? null;
    const status = validateNativeRedemption(row, identity.managerId, redemption, originals?.get(row.operation_id));
    if (status === 'Queued' || row.outcome === 'NativeQueued' || (originals && !originals.has(row.operation_id))) {
      throw new Error('HOLD: original native operation is not terminal');
    }
    return { operationId: row.operation_id, logIndex: BigInt(row.log_index), child: row.child!,
      redemption: { ...redemption!, status } };
  });
}


export const BridgingRequestedAbi = [{ type: 'event', name: 'BridgingRequested', anonymous: false, inputs: [
  { name: 'from', type: 'address', indexed: true }, { name: 'to', type: 'bytes32', indexed: true },
  { name: 'token', type: 'address', indexed: true }, { name: 'amount', type: 'uint256', indexed: false },
] }] as const;

/** Compare durable settled rows to every original authenticated manager log, not balances or proxy success. */
export function validateInboundTokenEffect(receiptRlp: Uint8Array, deposits: readonly SettledReceiptDeposit[], expected: InboundTokenEffect): void {
  const envelope = decodeRlp(receiptRlp);
  if (bytesToHex(encodeRlp(envelope)) !== bytesToHex(receiptRlp)) throw new Error('Noncanonical original receipt envelope');
  let receipt = envelope;
  if (envelope instanceof Uint8Array) {
    if (![1, 2, 3, 4].includes(envelope[0])) throw new Error('Unsupported original receipt envelope');
    const payload = envelope.subarray(1);
    receipt = decodeRlp(payload);
    if (bytesToHex(encodeRlp(receipt)) !== bytesToHex(payload)) throw new Error('Noncanonical original typed receipt');
  }
  if (!Array.isArray(receipt) || receipt.length !== 4 ||
      !(receipt[0] instanceof Uint8Array) || bytesToHex(receipt[0]) !== '0x01' || !Array.isArray(receipt[3])) {
    throw new Error('Invalid successful original receipt frame');
  }
  const topic = keccak256(stringToHex('BridgingRequested(address,bytes32,address,uint256)'));
  let matched = 0, expectedMatches = 0;

  for (const [index, log] of receipt[3].entries()) {
    if (!Array.isArray(log) || log.length !== 3 || !(log[0] instanceof Uint8Array) || !Array.isArray(log[1]) ||
        !(log[2] instanceof Uint8Array) || !log[1].every(item => item instanceof Uint8Array)) {
      throw new Error('Malformed original receipt log');
    }
    const topics = (log[1] as Uint8Array[]).map(item => bytesToHex(item));
    if (bytesToHex(log[0]).toLowerCase() !== expected.managerAddress.toLowerCase() || topics[0] !== topic) continue;
    const { args } = decodeEventLog({ abi: BridgingRequestedAbi, data: bytesToHex(log[2]), topics: topics as [HexString, ...HexString[]], strict: true });
    const rows = deposits.filter(row => BigInt(row.log_index) === BigInt(index));
    if (rows.length !== 1) throw new Error('Missing or ambiguous original receipt deposit');
    const row = rows[0];
    if (row.outcome !== 'Settled' || row.sender.toLowerCase() !== args.from.toLowerCase() ||
        row.receiver.toLowerCase() !== args.to.toLowerCase() || row.eth_token_id.toLowerCase() !== args.token.toLowerCase() ||
        BigInt(row.amount) !== args.amount) throw new Error('Original receipt deposit is not the exact settled effect');
    matched++;
    if (args.from.toLowerCase() === expected.sender.toLowerCase() && args.to.toLowerCase() === expected.receiver.toLowerCase() &&
        args.token.toLowerCase() === expected.sourceToken.toLowerCase() && row.token_id.toLowerCase() === expected.destinationToken.toLowerCase() &&
        args.amount === expected.amount) expectedMatches++;
  }
  if (!matched || matched !== deposits.length || expectedMatches < 1) throw new Error('Original receipt does not prove the expected token effect');
}
