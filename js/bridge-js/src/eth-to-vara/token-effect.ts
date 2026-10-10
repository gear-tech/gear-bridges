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

export const VFT_MANAGER_IDL_SHA256 = 'ceb2235140a76a6fb289c0673e4b4f17a2907acc50103f570218c333b527d683';
export const NATIVE_WRAPPER_IDL_SHA256 = 'eb23027b40bae7325943d6dd5ba419f99a084aa281681003ac4d8330bc770f3e';

export interface NativeReceiptDeposit extends SettledReceiptDeposit {
  supply: 'Gear' | 'Ethereum'; native: boolean; operation_id: HexString; child: HexString | null;
}
export interface NativeRedemption {
  from: HexString; to: HexString; amount: number | string | bigint; child: HexString;
  status: 'Queued' | 'Delivered' | 'Returned' | 'Ambiguous'; returned_value: number | string | bigint;
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
    if (row.outcome !== 'Settled' && !(row.native && row.outcome === 'NativeQueued')) throw new Error('HOLD: incomplete original native cohort');
    if (row.operation_id !== nativeOperationId(identity, BigInt(row.log_index))) throw new Error('HOLD: substituted original operation ID');
    if (row.native) {
      if (row.supply !== 'Gear' || row.token_id !== identity.wrapperId || !messageId(row.child)) throw new Error('HOLD: wrong native wrapper or original economic child');
      native++;
    }
  }
  if (!native) throw new Error('HOLD: native-pending reply lacks its original native operation');
}

/** Only original NativeQueued -> Settled is allowed; every economic identity and child remains fixed. */
export function validateNativeCohortContinuity(original: readonly NativeReceiptDeposit[], current: readonly NativeReceiptDeposit[], settled: boolean): void {
  if (current.length !== original.length) throw new Error('HOLD: native cohort row count changed');
  original.forEach((before, index) => {
    const after = current[index];
    if (BigInt(before.log_index) !== BigInt(after.log_index) || before.sender !== after.sender || before.receiver !== after.receiver ||
        before.eth_token_id !== after.eth_token_id || before.token_id !== after.token_id || BigInt(before.amount) !== BigInt(after.amount) ||
        before.supply !== after.supply || before.native !== after.native || before.operation_id !== after.operation_id || before.child !== after.child ||
        (after.outcome !== 'Settled' && !(before.outcome === 'NativeQueued' && after.outcome === 'NativeQueued' && !settled))) {
      throw new Error('HOLD: original native cohort identity or outcome changed');
    }
  });
}

/** Manager->wrapper child and wrapper->recipient payout child are different immutable identities. */
export function validateNativeRedemption(row: NativeReceiptDeposit, managerId: HexString, value: NativeRedemption | null, original?: NativeRedemption): boolean {
  if (!value || value.from !== managerId || value.to !== row.receiver || BigInt(value.amount) !== BigInt(row.amount) || !messageId(value.child) || value.child === row.child ||
      BigInt(value.returned_value) !== 0n || (value.status !== 'Queued' && value.status !== 'Delivered') ||
      (original && value.child !== original.child) || (row.outcome === 'Settled' && value.status !== 'Delivered')) {
    throw new Error('HOLD: native payout missing, returned, ambiguous or substituted');
  }
  return value.status === 'Delivered';
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
