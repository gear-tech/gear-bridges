import type { GearApi, HexString } from '@gear-js/api';
import { keccak256 } from 'viem';
import { HistoricalProxyClient } from '../vara/historical-proxy.js';
import { authenticateInboundSource, immutableInboundProfile, type InboundProofProfile } from './proof-composer.js';

const authenticatedReceipts = new WeakSet<object>();
export type HistoricalReceiptAuthority = Readonly<{
  framing: 'network-rlp'; receiptHash: HexString; managerId: HexString; proxyId: HexString; slot: bigint; transactionIndex: bigint;
}>;

export function assertHistoricalReceiptAuthority(authority: HistoricalReceiptAuthority, receipt: Uint8Array): void {
  if (!authenticatedReceipts.has(authority) || keccak256(receipt) !== authority.receiptHash) throw new Error('HOLD: historical receipt authority is missing or belongs to different original bytes');
}

export type HistoricalReceiptOriginal = Readonly<{
  api: GearApi; preparedBlockHash: HexString; receiptHash: HexString; slot: bigint; transactionIndex: bigint;
}>;

/** Reconciliation only: authenticate the old deployment at the original finalized preparation pin. */
export async function authenticateHistoricalReceipt(profile: InboundProofProfile, original: HistoricalReceiptOriginal,
  receipt: Uint8Array, slot: bigint, transactionIndex: bigint): Promise<HistoricalReceiptAuthority> {
  profile = immutableInboundProfile(profile);
  if (slot !== original.slot || transactionIndex !== original.transactionIndex || keccak256(receipt) !== original.receiptHash.toLowerCase() ||
      slot < 0n || slot > BigInt(Number.MAX_SAFE_INTEGER)) throw new Error('HOLD: historical receipt original identity changed');
  const { api, preparedBlockHash: pin } = original;
  await authenticateInboundSource(api, profile, pin);
  const proxy = await api.programStorage.getProgram(profile.historicalProxyId, pin);
  if (!proxy.state.isInitialized || !proxy.codeId.eq(profile.historicalProxyCodeId)) throw new Error('HOLD: historical proxy identity mismatch');
  const endpoint = await new HistoricalProxyClient(api, profile.historicalProxyId).historicalProxy.endpointFor(Number(slot)).atBlock(pin).call();
  if (!('ok' in endpoint)) throw new Error('HOLD: historical receipt endpoint unavailable');
  const [manager, deployedEndpoint] = await Promise.all([
    api.programStorage.getProgram(profile.consumer.programId, pin), api.programStorage.getProgram(endpoint.ok, pin),
  ]);
  const approvals = profile.historicalReceipts?.filter(row => row.managerId === profile.consumer.programId && row.managerCodeId === profile.consumer.codeId &&
    row.endpointId === endpoint.ok.toLowerCase() && row.endpointId === profile.endpoint.programId && row.endpointCodeId === profile.endpoint.codeId &&
    slot >= row.minimumSlot && slot <= row.maximumSlot) ?? [];
  if (approvals.length !== 1 || !manager.state.isInitialized || !manager.codeId.eq(approvals[0].managerCodeId) ||
      !deployedEndpoint.state.isInitialized || !deployedEndpoint.codeId.eq(approvals[0].endpointCodeId)) throw new Error('HOLD: historical receipt code/framing lacks exact independent approval');
  if (keccak256(receipt) !== original.receiptHash.toLowerCase()) throw new Error('HOLD: historical receipt bytes changed during qualification');
  const authority = Object.freeze({ framing: approvals[0].framing, receiptHash: keccak256(receipt), managerId: profile.consumer.programId, proxyId: profile.historicalProxyId, slot, transactionIndex });
  authenticatedReceipts.add(authority);
  return authority;
}
