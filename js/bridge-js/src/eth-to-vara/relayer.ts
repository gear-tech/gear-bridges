import type { KeyringPair } from '@polkadot/keyring/types';
import { SignerOptions } from '@polkadot/api/types';
import { GearApi, HexString, decodeAddress } from '@gear-js/api';
import { Header } from '@polkadot/types/interfaces';
import type { Enum } from '@polkadot/types';
import { TypeRegistry } from '@polkadot/types';
import type { ITuple } from '@polkadot/types/types';
import { getServiceNamePrefix, getFnNamePrefix, TransactionBuilder } from 'sails-js';
import { PublicClient, bytesToHex } from 'viem';

import { encodeEthToVaraEvent, getPrefix, GearClient, HistoricalProxyClient, ProxyError, ProofResult } from '../vara/index.js';
import { createBeaconClient, createEthereumClient } from '../ethereum/index.js';
import { composeProof, InboundProofProfile, immutableInboundProfile, authenticateInboundSource } from './proof-composer.js';
import { StatusCb, withOriginalDeadline } from '../util.js';
import { authenticateHistoricalReceipt, type HistoricalReceiptOriginal } from './historical-receipt.js';
import { VFT_MANAGER_IDL_SHA256, validateInboundTokenEffect, validateNativePendingCohort, validateNativeCohortContinuity, validateNativeRedemption, validateNativeTokenEffect,
  type NativeReceiptDeposit, type NativeRedemption, type NativeReceiptIdentity, type InboundTokenEffect, type NativeTokenSettlement } from './token-effect.js';

export interface RelayResult {
  blockHash: HexString;
  msgId: HexString;
  txHash: HexString;
  isFinalized: Promise<boolean>;
  replyPayload: HexString;
  replyBlockHash: HexString;
  replyBlockNumber: bigint;
  replyMessageId: HexString;
  clientReply?: HexString;
  error?: ProxyError;
  /** Original reply is retained; a native completion may have a later separately authenticated effect pin. */
  effectBlockHash?: HexString; effectBlockNumber?: bigint; nativeReconciliation?: RelayResult;
  /** Terminal original payouts: Refunded restores wrapped tokens to the original recipient, not native delivery. */
  nativeSettlements?: readonly NativeTokenSettlement[];
}
export interface ConsumerReplyContract {
  readonly reconciliationOnly?: boolean;
  readonly registry: TypeRegistry;
  readonly nativeSettlement?: Readonly<{
    expectedEffect: InboundTokenEffect;
    readDeposits: (pin: HexString, proof: ProofResult) => Promise<readonly NativeReceiptDeposit[]>;
    readRedemption: (operationId: HexString, pin: HexString) => Promise<NativeRedemption | null>;
    readReceiptStatus: (pin: HexString, proof: ProofResult) => Promise<'Unknown' | 'Reserved' | 'Processed' | 'Resolved'>;
  }>;
  readonly resultType: string;
  readonly idlSha256: string;
  /** Must verify exact application/economic state at result.effectBlockHash (or original reply pin). */
  readonly verifyEffect: (value: unknown, result: RelayResult, proof: ProofResult) => Promise<void>;
}

function nativeReceiptIdentity(profile: InboundProofProfile, proof: ProofResult, expectedEffect: InboundTokenEffect): NativeReceiptIdentity {
  return { managerId: profile.consumer.programId, proxyId: profile.historicalProxyId,
    wrapperId: profile.nativeWrapper!.programId, wrapperMode: profile.nativeWrapper!.mode,
    wrapperEthereumToken: profile.nativeWrapper!.ethereumToken, slot: BigInt(proof.proofBlock.block.slot),
    transactionIndex: BigInt(proof.transactionIndex), receiptRlp: proof.receiptRlp, expectedEffect };
}

async function readNativeRedemptions(rows: readonly NativeReceiptDeposit[], pin: HexString,
  readRedemption: NonNullable<ConsumerReplyContract['nativeSettlement']>['readRedemption']) {
  const redemptions = new Map<HexString, NativeRedemption | null>();
  for (const row of rows.filter(row => row.native)) {
    redemptions.set(row.operation_id.toLowerCase() as HexString, await readRedemption(row.operation_id, pin));
  }
  return redemptions;
}

/** Standard manager adapter; arbitrary consumers must still supply their own verifyEffect. */
export function createVftManagerReplyContract(profile: InboundProofProfile,
  adapter: Pick<ConsumerReplyContract, 'registry' | 'idlSha256'> & NonNullable<ConsumerReplyContract['nativeSettlement']> & { historicalOriginal?: HistoricalReceiptOriginal }): ConsumerReplyContract {
  profile = immutableInboundProfile(profile);
  if (profile.consumer.service !== 'VftManager' || profile.consumer.method !== 'SubmitReceipt' ||
      adapter.idlSha256.toLowerCase() !== profile.consumer.idlSha256 ||
      (!adapter.historicalOriginal && adapter.idlSha256.toLowerCase() !== VFT_MANAGER_IDL_SHA256)) {
    throw new Error('HOLD: consumer decoder does not match approved deployed IDL');
  }
  if (profile.nativeWrapper &&
      (adapter.expectedEffect.destinationToken.toLowerCase() === profile.nativeWrapper.programId) !==
      (adapter.expectedEffect.sourceToken.toLowerCase() === profile.nativeWrapper.ethereumToken)) {
    throw new Error('HOLD: native token mapping differs from approved wrapper binding');
  }
  const { registry, idlSha256, historicalOriginal: suppliedOriginal, ...suppliedSettlement } = adapter;
  const historicalOriginal = suppliedOriginal && Object.freeze({ ...suppliedOriginal });
  const settlement = { ...suppliedSettlement, expectedEffect: Object.freeze({ ...suppliedSettlement.expectedEffect }) };
  return {
    registry, idlSha256: idlSha256.toLowerCase(), resultType: 'Result<Null, Error>', reconciliationOnly: !!historicalOriginal,
    nativeSettlement: profile.nativeWrapper ? settlement : undefined,
    verifyEffect: async (_value, result, proof) => {
      const pin = result.effectBlockHash ?? result.replyBlockHash;
      const framing = historicalOriginal ? await authenticateHistoricalReceipt(profile, historicalOriginal, proof.receiptRlp, BigInt(proof.proofBlock.block.slot), BigInt(proof.transactionIndex)) : undefined;
      if (historicalOriginal) await authenticateEffectProfile(historicalOriginal.api, profile, pin);
      const deposits = await settlement.readDeposits(pin, proof);
      if (!deposits.some(deposit => deposit.native)) {
        validateInboundTokenEffect(proof.receiptRlp, deposits, settlement.expectedEffect, framing);
        return;
      }
      if (!profile.nativeWrapper) throw new Error('HOLD: native wrapper identity is unavailable is not defined');
      const identity = { ...nativeReceiptIdentity(profile, proof, settlement.expectedEffect), historicalReceipt: framing };
      const redemptions = await readNativeRedemptions(deposits, pin, settlement.readRedemption);
      if (historicalOriginal && !result.nativeSettlements) {
        await authenticateEffectProfile(historicalOriginal.api, profile, result.replyBlockHash);
        for (const at of new Set([result.replyBlockHash, pin])) {
          const wrapper = await historicalOriginal.api.programStorage.getProgram(profile.nativeWrapper.programId, at);
          if (!wrapper.state.isInitialized || !wrapper.codeId.eq(profile.nativeWrapper.codeId)) throw new Error('HOLD: historical native wrapper code mismatch');
        }
        const originalRows = structuredClone(await settlement.readDeposits(result.replyBlockHash, proof));
        validateNativePendingCohort(identity, originalRows);
        validateNativeCohortContinuity(originalRows, deposits, true);
        const originalRedemptions = await readNativeRedemptions(originalRows, result.replyBlockHash, settlement.readRedemption);
        const authenticated = new Map<HexString, NativeRedemption>();
        for (const row of originalRows.filter(row => row.native)) {
          const redemption = originalRedemptions.get(row.operation_id);
          validateNativeRedemption(row, identity.managerId, identity.wrapperMode, redemption ?? null);
          authenticated.set(row.operation_id, redemption!);
        }
        result.nativeSettlements = validateNativeTokenEffect(identity, deposits, redemptions, authenticated);
      }
      if (!result.nativeSettlements) throw new Error('HOLD: authenticated original native settlements are missing is not defined');
      const originals = new Map(result.nativeSettlements.map(entry => [entry.operationId, entry.redemption]));
      const settlements = validateNativeTokenEffect(identity, deposits, redemptions, originals);
      if (settlements.length !== result.nativeSettlements.length ||
          settlements.some(entry => entry.child.toLowerCase() !== result.nativeSettlements!.find(original =>
            original.operationId.toLowerCase() === entry.operationId.toLowerCase())?.child.toLowerCase())) {
        throw new Error('HOLD: original native settlement cohort changed');
      }
    },
  };
}

function consumerResult(payload: HexString, profile: InboundProofProfile, contract: Pick<ConsumerReplyContract, 'registry' | 'resultType' | 'idlSha256'>) {
  profile = immutableInboundProfile(profile);
  if (contract.idlSha256.toLowerCase() !== profile.consumer.idlSha256 || !/^Result<.+>$/.test(contract.resultType)) {
    throw new Error('HOLD: consumer decoder does not match approved deployed IDL');
  }
  const decoded = contract.registry.createType<ITuple>(
    '(String, String, ' + contract.resultType + ')', payload);
  if (decoded.toHex() !== payload.toLowerCase() || decoded[0].toString() !== profile.consumer.service ||
      decoded[1].toString() !== profile.consumer.method) throw new Error('Malformed or wrong-route consumer reply');
  return decoded[2] as unknown as { isErr: boolean; isOk: boolean; asErr: Enum; asOk: { toJSON(): unknown } };
}
export function decodeConsumerReply(payload: HexString, profile: InboundProofProfile, contract: Pick<ConsumerReplyContract, 'registry' | 'resultType' | 'idlSha256'>): unknown {
  const result = consumerResult(payload, profile, contract);
  if (!result.isOk || result.isErr) throw new Error('Finalized consumer error: ' + result.asErr.toString());
  return result.asOk.toJSON();
}
async function authenticateEffectProfile(api: GearApi, profile: InboundProofProfile, pin: HexString): Promise<void> {
  await authenticateInboundSource(api, profile, pin);
  const consumer = await api.programStorage.getProgram(profile.consumer.programId, pin);
  if (!consumer.state.isInitialized || !consumer.codeId.eq(profile.consumer.codeId)) {
    throw new Error('HOLD: finalized effect runtime or consumer differs from immutable approval');
  }
}


/** This is a pending classifier, never an application-success decoder. */
export function assertNativePendingReply(payload: HexString, profile: InboundProofProfile, contract: ConsumerReplyContract): void {
  profile = immutableInboundProfile(profile);
  const result = consumerResult(payload, profile, contract);
  if (!result.isErr || result.isOk || result.asErr.type !== 'NativeSettlementPending' ||
      profile.consumer.service !== 'VftManager' || profile.consumer.method !== 'SubmitReceipt' ||
      profile.consumer.idlSha256 !== VFT_MANAGER_IDL_SHA256 || contract.resultType !== 'Result<Null, Error>' ||
      !profile.nativeWrapper ||
      !contract.nativeSettlement || typeof contract.nativeSettlement.readDeposits !== 'function' || typeof contract.nativeSettlement.readRedemption !== 'function' ||
      typeof contract.nativeSettlement.readReceiptStatus !== 'function') {
    throw new Error('Finalized consumer error cannot reconcile: ' + (result.isErr ? result.asErr.toString() : 'not native pending'));
  }
}

async function reconcileOriginalNativeSettlement(params: RelayEthToVaraParams, proof: ProofResult, original: RelayResult, deadline: number): Promise<void> {
  const api = params.gearApi, profile = params.inboundProfile, contract = params.consumerReply.nativeSettlement!;
  const read = <T>(operation: () => Promise<T>) => withOriginalDeadline(deadline, operation);
  const identity = nativeReceiptIdentity(profile, proof, contract.expectedEffect);
  const originalWrapper = await read(() => api.programStorage.getProgram(identity.wrapperId, original.replyBlockHash));
  if (!originalWrapper.state.isInitialized || !originalWrapper.codeId.eq(profile.nativeWrapper!.codeId)) throw new Error('HOLD: original native wrapper identity changed');
  const rows = structuredClone(await read(() => contract.readDeposits(original.replyBlockHash, proof)));
  validateNativePendingCohort(identity, rows);
  const redemptions = new Map<HexString, NativeRedemption>();
  for (const row of rows.filter(row => row.native)) {
    const value = await read(() => contract.readRedemption(row.operation_id, original.replyBlockHash));
    validateNativeRedemption(row, identity.managerId, identity.wrapperMode, value);
    redemptions.set(row.operation_id.toLowerCase() as HexString, structuredClone(value!));
  }
  let previous = { number: original.replyBlockNumber, hash: original.replyBlockHash };
  let observedRows = rows;
  const verifySettlement = async (pin: HexString, settled: boolean) => {
    const current = structuredClone(await read(() => contract.readDeposits(pin, proof)));
    validateNativeCohortContinuity(observedRows, current, settled);
    let ready = true;
    const currentRedemptions = new Map<HexString, NativeRedemption>();
    for (const row of current.filter(row => row.native)) {
      const value = await read(() => contract.readRedemption(row.operation_id, pin));
      const status = validateNativeRedemption(row, identity.managerId, identity.wrapperMode, value, redemptions.get(row.operation_id.toLowerCase() as HexString));
      if (status === 'Queued' && BigInt(value!.returned_value) === 0n) ready = false;
      currentRedemptions.set(row.operation_id.toLowerCase() as HexString, structuredClone(value!));
    }
    const wrapper = await read(() => api.programStorage.getProgram(identity.wrapperId, pin));
    const manager = await read(() => api.programStorage.getProgram(identity.managerId, pin));
    if (!wrapper.state.isInitialized || !wrapper.codeId.eq(profile.nativeWrapper!.codeId) || !manager.state.isInitialized || !manager.codeId.eq(profile.consumer.codeId)) {
      throw new Error('HOLD: native continuation actor identity changed');
    }
    observedRows = current;
    for (const [operation, value] of currentRedemptions) redemptions.set(operation, value);
    if (settled) original.nativeSettlements = validateNativeTokenEffect(identity, current, currentRedemptions);
    return ready;
  };
  while (true) {
    const tipHash = (await read(() => api.rpc.chain.getFinalizedHead())).toHex();
    const tip = await read(() => api.rpc.chain.getHeader(tipHash)), number = tip.number.toBigInt();
    if (number < previous.number || (await read(() => api.blocks.getBlockHash(previous.number))).toHex() !== previous.hash) throw new Error('HOLD: native finalized ancestry changed');
    for (let next = previous.number + 1n; next <= number; next++) {
      const hash = (await read(() => api.blocks.getBlockHash(next))).toHex(), header = await read(() => api.rpc.chain.getHeader(hash));
      if (header.number.toBigInt() !== next || header.parentHash.toHex() !== previous.hash) throw new Error('HOLD: native finalized ancestry gap');
      previous = { number: next, hash };
    }
    if (previous.hash !== tipHash) throw new Error('HOLD: native finalized head mismatch');
    if (await verifySettlement(tipHash, false)) {
      const status = await read(() => contract.readReceiptStatus(tipHash, proof));
      if (status === 'Unknown') throw new Error('HOLD: native receipt status is unknown');
      if (status === 'Processed') {
        if (!await verifySettlement(tipHash, true)) throw new Error('HOLD: processed native receipt lacks original terminal settlement');
        // The shared completion guard below authenticates this exact effect pin.
        original.effectBlockHash = tipHash; original.effectBlockNumber = number; return;
      }
      if (status !== 'Reserved') throw new Error('HOLD: unsupported native receipt status');
      break;
    }
    params.statusCb?.('Native payout pending; waiting for original delivery or authenticated full return before reconciliation', { originalMessageId: original.msgId });
    await read(() => new Promise<void>(resolve => setTimeout(resolve, 3000)));
  }
  await read(() => authenticateEffectProfile(api, profile, previous.hash));
  params.statusCb?.('Signing separate non-economic receipt reconciliation', { originalMessageId: original.msgId, originalTransactionHash: original.txHash });
  const transaction = new TransactionBuilder(api, params.consumerReply.registry, 'send_message', 'VftManager', 'ReconcileReceipt',
    [identity.slot, identity.transactionIndex], '(u64, u64)', 'Result<ReceiptStatus, Error>', identity.managerId);
  // This is a new non-economic command, never a replacement nonce or proof submission.
  const signerOptions = { ...params.signerOptions, nonce: -1 };
  transaction.withAccount(params.signer, signerOptions);
  await read(() => transaction.calculateGas());
  const ceiling = api.blockGasLimit.toBigInt() / 100n * 95n;
  if (transaction.gasInfo.min_limit.toBigInt() > ceiling) throw new Error('Native reconciliation exceeds outer gas ceiling');
  transaction.withValue(0n);
  const payload = transaction.extrinsic.args[1].toHex();
  await read(() => transaction.extrinsic.signAsync(params.signer, signerOptions));
  if (Date.now() >= deadline) throw new Error('Original native settlement deadline expired before reconciliation');
  const txHash = transaction.extrinsic.hash.toHex();
  let unsubscribe: (() => void) | undefined, finished = false;
  try {
    const queued = await read(() => new Promise<{ blockHash: HexString; msgId: HexString }>((resolve, reject) => {
      transaction.extrinsic.send(({ events, status }) => {
        if (finished) return;
        if (status.isInvalid || status.isDropped || status.isUsurped || status.isRetracted) { finished = true; reject(new Error('Original non-economic reconciliation transaction failed')); return; }
        if (!status.isInBlock) return;
        const failed = events.find(({ event }) => api.events.system.ExtrinsicFailed.is(event));
        if (failed) {
          const error = api.getExtrinsicFailedError(failed.event);
          finished = true; reject(new Error(`Original reconciliation transaction ${txHash}: ${error.method}: ${error.docs}`)); return;
        }
        const messages = events.filter(({ event }) => api.events.gear.MessageQueued.is(event));
        if (messages.length !== 1 || !events.some(({ event }) => api.events.system.ExtrinsicSuccess.is(event)) || events.some(({ event }) => api.events.system.ExtrinsicFailed.is(event))) {
          finished = true; reject(new Error('Original reconciliation did not queue exactly one successful message')); return;
        }
        finished = true; resolve({ blockHash: status.asInBlock.toHex(), msgId: messages[0].event.data[0].toHex() });
      }).then(stop => { unsubscribe = stop; if (finished) stop(); }).catch(reject);
    }));
    const reconciliation = await read(() => validateFinalizedNativeReconciliationReply({ gearApi: api, inboundProfile: profile, registry: params.consumerReply.registry,
      slot: identity.slot, transactionIndex: identity.transactionIndex, sender: decodeAddress(typeof params.signer === 'string' ? params.signer : params.signer.address),
      ...queued, txHash, requestPayload: payload, deadline, statusCb: params.statusCb }));
    if (!await verifySettlement(reconciliation.replyBlockHash, true)) throw new Error('HOLD: final reconciliation lacks original terminal settlement');
    original.nativeReconciliation = reconciliation;
    original.effectBlockHash = reconciliation.replyBlockHash; original.effectBlockNumber = reconciliation.replyBlockNumber;
  } finally { finished = true; unsubscribe?.(); }
}


/**
 * Parameters for relaying an Ethereum transaction to the Vara network.
 * This interface defines all the required configuration and optional settings
 * needed to relay cross-chain transactions from Ethereum to Vara.
 */
export type RelayEthToVaraParams = {
  /**
   * Transaction hash of the Ethereum transaction to relay
   */
  transactionHash: `0x${string}`;
  /**
   * The RPC URL for the Ethereum beacon chain client
   */
  beaconRpcUrl: string;
  /**
   * Viem public client for Ethereum network interactions
   */
  ethereumPublicClient: PublicClient;
  /**
   * Gear API instance for Vara network operations
   */
  gearApi: GearApi;
  /**
   * ID of the historical proxy program on Vara
   */
  historicalProxyId: `0x${string}`;
  inboundProfile: InboundProofProfile;
  consumerReply: ConsumerReplyContract;
  /**
   * ID of the target client program on Vara
   */
  clientId: `0x${string}`;
  /**
   * Name of the service to call on the target client
   */
  clientServiceName: string;
  /**
   * Name of the method to call on the target service
   */
  clientMethodName: string;
  /**
   * Flag indicating whether to wait for the slot to appear on the CheckpointClient contract
   */
  wait?: boolean;
  /**
   * Account signer, either as string address or KeyringPair for transaction signing
   */
  signer: string | KeyringPair;
  /**
   * Optional signing configuration parameters
   */
  signerOptions?: Partial<SignerOptions>;
  /**
   * Callback function to track the status of the transaction
   */
  statusCb?: StatusCb;
  deadline?: number;
};

export type PrepareEthToVaraParams = Omit<RelayEthToVaraParams, 'signer' | 'signerOptions' | 'consumerReply'>;

export type PreparedEthToVaraRelay = {
  transaction: ReturnType<HistoricalProxyClient['historicalProxy']['redirect']>;
  proof: ProofResult;
};

/** Package-internal unsigned preparation, shared by SDK relaying and durable examples. */
export function prepareEthToVaraRelay(params: PrepareEthToVaraParams): Promise<PreparedEthToVaraRelay> {
  const deadline = params.deadline ?? Date.now() + 44 * 60 * 1000;
  const inboundProfile = immutableInboundProfile(params.inboundProfile);
  return withOriginalDeadline(deadline, () => prepareOriginalEthToVaraRelay({ ...params, inboundProfile, deadline }));
}

async function prepareOriginalEthToVaraRelay(params: PrepareEthToVaraParams & { deadline: number }): Promise<PreparedEthToVaraRelay> {
  const statusCb = params.statusCb ?? (() => {});
  const deadline = params.deadline;
  const read = <T>(operation: () => Promise<T>) => withOriginalDeadline(deadline, operation);
  const profile = params.inboundProfile;
  if ((profile.consumer.service === 'VftManager' && profile.consumer.idlSha256 !== VFT_MANAGER_IDL_SHA256) ||
      profile.historicalReceipts?.some(row => row.managerId === profile.consumer.programId && row.managerCodeId === profile.consumer.codeId && row.endpointId === profile.endpoint.programId && row.endpointCodeId === profile.endpoint.codeId)) throw new Error('HOLD: historical manager profile is reconciliation-only');
  if (params.historicalProxyId.toLowerCase() !== profile.historicalProxyId.toLowerCase() ||
      params.clientId.toLowerCase() !== profile.consumer.programId.toLowerCase() ||
      params.clientServiceName !== profile.consumer.service || params.clientMethodName !== profile.consumer.method) {
    throw new Error('HOLD: requested consumer or proxy differs from approved inbound profile');
  }
  const beaconClient = await read(() => createBeaconClient(params.beaconRpcUrl, deadline));
  const ethClient = createEthereumClient(params.ethereumPublicClient, beaconClient);
  const proxy = new HistoricalProxyClient(params.gearApi, params.historicalProxyId);
  statusCb('Composing proof', { txHash: params.transactionHash });

  const [chainId, genesis, receipt, finalized] = await read(() => Promise.all([
    params.ethereumPublicClient.getChainId(), params.ethereumPublicClient.getBlock({ blockNumber: 0n }),
    params.ethereumPublicClient.getTransactionReceipt({ hash: params.transactionHash }),
    params.ethereumPublicClient.getBlock({ blockTag: 'finalized' }),
  ]));
  if (BigInt(chainId) !== profile.ethereumChainId || genesis.hash.toLowerCase() !== profile.ethereumGenesisHash.toLowerCase()) {
    throw new Error('Inbound proof Ethereum chain or execution genesis mismatch');
  }
  if (receipt.blockNumber > finalized.number ||
      (await read(() => params.ethereumPublicClient.getBlock({ blockNumber: receipt.blockNumber }))).hash !== receipt.blockHash) {
    throw new Error('Original Ethereum receipt is not canonical finalized history');
  }
  const proof = await composeProof(beaconClient, ethClient, proxy, params.transactionHash, profile, params.wait ?? false, statusCb, deadline);
  if ((await read(() => params.ethereumPublicClient.getBlock({ blockNumber: receipt.blockNumber }))).hash !== receipt.blockHash ||
      (await read(() => params.ethereumPublicClient.getBlock({ blockNumber: finalized.number }))).hash !== finalized.hash) {
    throw new Error('Original finalized Ethereum preparation history changed during Beacon proof I/O');
  }
  if (Date.now() >= deadline) throw new Error('Original relay deadline expired during proof preparation');
  const transaction = proxy.historicalProxy.redirect(
    proof.proofBlock.block.slot,
    encodeEthToVaraEvent(proof),
    params.clientId,
    getPrefix(params.clientServiceName, params.clientMethodName),
  );
  return { transaction, proof };
}

export type FinalizedEthToVaraReplyParams = {
  gearApi: GearApi;
  historicalProxyId: HexString;
  sender: HexString;
  msgId: HexString;
  blockHash: HexString;
  txHash: HexString;
  requestPayload: HexString;
  receiptRlp: Uint8Array;
  /** Absolute original deadline, also on resume. */
  deadline: number;
  statusCb?: StatusCb;
};

/** Validate the original request, then observe only its canonically finalized reply. */
export function validateFinalizedEthToVaraReply(params: FinalizedEthToVaraReplyParams): Promise<RelayResult> {
  return withOriginalDeadline(params.deadline, () => validateOriginalFinalizedEthToVaraReply(params));
}
export type FinalizedNativeReconciliationReplyParams = Omit<FinalizedEthToVaraReplyParams, 'historicalProxyId' | 'receiptRlp'> & {
  inboundProfile: InboundProofProfile; registry: TypeRegistry; slot: bigint; transactionIndex: bigint;
};
/** Separate original NON-economic command; never replace the proof request or its reply. */
export function validateFinalizedNativeReconciliationReply(params: FinalizedNativeReconciliationReplyParams): Promise<RelayResult> {
  const profile = immutableInboundProfile(params.inboundProfile);
  if (profile.consumer.idlSha256 !== VFT_MANAGER_IDL_SHA256 || profile.consumer.service !== 'VftManager' || profile.consumer.method !== 'SubmitReceipt') {
    throw new Error('HOLD: unqualified native reconciliation consumer');
  }
  const request = params.registry.createType('(String, String, u64, u64)', params.requestPayload);
  if (request.toHex() !== params.requestPayload.toLowerCase() || request[0].toString() !== 'VftManager' || request[1].toString() !== 'ReconcileReceipt' ||
      request[2].toString() !== params.slot.toString() || request[3].toString() !== params.transactionIndex.toString()) throw new Error('HOLD: native reconciliation coordinates differ from original receipt');
  return withOriginalDeadline(params.deadline, () => validateOriginalFinalizedEthToVaraReply({ ...params,
    historicalProxyId: profile.consumer.programId, receiptRlp: new Uint8Array() }, params.registry));
}


async function validateOriginalFinalizedEthToVaraReply(params: FinalizedEthToVaraReplyParams, reconciliationRegistry?: TypeRegistry): Promise<RelayResult> {
  const { gearApi: api, historicalProxyId, sender, msgId, blockHash, txHash, requestPayload, deadline } = params;
  const registry = reconciliationRegistry ?? new HistoricalProxyClient(api, historicalProxyId).registry;
  const service = reconciliationRegistry ? 'VftManager' : 'HistoricalProxy', method = reconciliationRegistry ? 'ReconcileReceipt' : 'Redirect';
  const request = registry.createType(reconciliationRegistry ? '(String, String, u64, u64)' : '(String, String, u64, Vec<u8>, [u8;32], Vec<u8>)', requestPayload);
  if (request.toHex() !== requestPayload.toLowerCase() || request[0].toString() !== service || request[1].toString() !== method) {
    throw new Error('Invalid original ' + service + '/' + method + ' request');
  }
  const originalBlock = await api.blocks.get(blockHash);
  const gearClient = new GearClient(api);
  const messageIds = new Set([msgId.toLowerCase()]);
  let next = originalBlock.block.header.number.toBigInt();
  let previousHash: HexString | undefined;

  return new Promise<RelayResult>((resolve, reject) => {
    let finished = false;
    let unsubscribe: (() => void) | undefined;
    let pending = Promise.resolve();
    const finish = (result?: RelayResult, error?: unknown) => {
      if (finished) return;
      finished = true;
      clearTimeout(timer);
      unsubscribe?.();
      if (error !== undefined) reject(error);
      else resolve(result!);
    };
    const timer = setTimeout(() => finish(undefined, new Error('Original relay deadline expired without a finalized reply')),
      deadline - Date.now());
    const scan = async (head: Header) => {
      if (finished) return;
      const tip = head.number.toBigInt();
      for await (const block of gearClient.finalizedReplyBlocks(next, tip, messageIds, deadline, previousHash, {
        deadline: 'Original relay deadline expired', history: 'Finalized source history changed or has a gap',
      })) {
        if (finished) return;
        const { number, hash, events } = block;
        next = number;
        if (previousHash === undefined) {
          if (hash !== blockHash.toLowerCase()) throw new Error('Original request inclusion is not canonical finalized history');
          const extrinsics = originalBlock.block.extrinsics;
          const indexes = extrinsics.flatMap((tx, index) => tx.hash.toHex() === txHash.toLowerCase() ? [index] : []);
          if (indexes.length !== 1) throw new Error('Original signed request extrinsic not found');
          const index = indexes[0];
          const tx = extrinsics[index];
          if (!tx.isSigned || api.createType('AccountId', tx.signer.toString()).toHex() !== sender.toLowerCase() || tx.method.section !== 'gear' ||
              tx.method.method !== 'sendMessage' || tx.method.args[0].toHex() !== historicalProxyId.toLowerCase() ||
              tx.method.args[1].toHex() !== requestPayload.toLowerCase() || tx.method.args[3].toString() !== '0') {
            throw new Error('Original signed request does not match sender, proxy, payload and zero value');
          }
          const requestEvents = events.filter(({ phase }) => phase.isApplyExtrinsic && phase.asApplyExtrinsic.toNumber() === index);
          const queued = requestEvents.filter(({ event }) => api.events.gear.MessageQueued.is(event) && event.data[0].eq(msgId));
          if (queued.length !== 1 || !requestEvents.some(({ event }) => api.events.system.ExtrinsicSuccess.is(event)) ||
              requestEvents.some(({ event }) => api.events.system.ExtrinsicFailed.is(event))) {
            throw new Error('Original request is not a successful finalized MessageQueued extrinsic');
          }
          const queuedEvent = queued[0].event;
          if (!api.events.gear.MessageQueued.is(queuedEvent) || !queuedEvent.data[1].eq(sender) ||
              !queuedEvent.data[2].eq(historicalProxyId)) {
            throw new Error('Original MessageQueued sender or proxy mismatch');
          }
        }
        const replies = [...block.replies()];
        if (replies.length > 1) throw new Error('Ambiguous original proxy reply');
        if (replies.length === 1) {
          const [message] = replies;
          if (!message.source.eq(historicalProxyId) || !message.destination.eq(sender)) {
            throw new Error('Original reply source or destination mismatch');
          }
          if (!message.details.unwrap().code.isSuccess) throw new Error('Original finalized reply has a runtime error code');
          const replyPayload = message.payload.toHex();
          const decoded = registry.createType(reconciliationRegistry ? '(String, String, Result<ReceiptStatus, Error>)' : '(String, String, Result<(Vec<u8>, Vec<u8>), ProxyError>)', message.payload);
          if (decoded.toHex() !== replyPayload || decoded[0].toString() !== service || decoded[1].toString() !== method) {
            throw new Error('Malformed or wrong-route original ' + service + '/' + method + ' reply');
          }
          if ((await api.blocks.getBlockHash(next)).toHex() !== hash) throw new Error('Original reply finalized pin changed');
          const result: RelayResult = {
            blockHash, msgId, txHash, isFinalized: Promise.resolve(true),
            replyPayload, replyBlockHash: hash, replyBlockNumber: next, replyMessageId: message.id.toHex(),
          };
          if (reconciliationRegistry) {
            if (!decoded[2].isOk || decoded[2].isErr || decoded[2].asOk.toString() !== 'Processed') throw new Error('HOLD: original native reconciliation did not return Processed');
            result.clientReply = replyPayload;
            finish(result); return;
          }
          if (decoded[2].isErr) {
            const error = decoded[2].asErr as Enum;
            result.error = { [error.type]: error.value.toString() } as ProxyError;
          } else {
            const [receipt, clientReply] = decoded[2].asOk as ITuple;
            if (bytesToHex(receipt.toU8a(true)) !== bytesToHex(params.receiptRlp)) {
              throw new Error('Original finalized proxy reply substituted the authenticated receipt');
            }
            result.clientReply = bytesToHex(clientReply.toU8a(true));
            if (getServiceNamePrefix(result.clientReply) !== getServiceNamePrefix(bytesToHex(request[5].toU8a(true))) ||
                getFnNamePrefix(result.clientReply) !== getFnNamePrefix(bytesToHex(request[5].toU8a(true)))) {
              throw new Error('Original consumer reply route differs from the original request');
            }
          }
          params.statusCb?.('Original proxy reply finalized', { msgId, replyBlockHash: hash });
          finish(result);
          return;
        }
        previousHash = hash;
        next++;
      }
    };
    const enqueue = (head: Header) => {
      pending = pending.then(() => scan(head)).catch((error) => finish(undefined, error));
    };
    void api.rpc.chain.subscribeFinalizedHeads(enqueue).then((stop) => {
      unsubscribe = stop;
      if (finished) stop();
      else void api.rpc.chain.getFinalizedHead().then((hash) => api.rpc.chain.getHeader(hash)).then(enqueue)
        .catch((error) => finish(undefined, error));
    }).catch((error) => finish(undefined, error));
  });
}

/** Returns raw client bytes only after the original request AND reply are finalized. */
export async function relayEthToVara(params: RelayEthToVaraParams): Promise<RelayResult> {
  const deadline = params.deadline ?? Date.now() + 44 * 60 * 1000;
  params = { ...params, inboundProfile: immutableInboundProfile(params.inboundProfile),
    consumerReply: params.consumerReply ? Object.freeze({ ...params.consumerReply,
      nativeSettlement: params.consumerReply.nativeSettlement ? Object.freeze({ ...params.consumerReply.nativeSettlement,
        expectedEffect: Object.freeze({ ...params.consumerReply.nativeSettlement.expectedEffect }) }) : undefined }) : params.consumerReply };
  if (params.consumerReply?.reconciliationOnly) throw new Error('HOLD: historical receipt adapter cannot sign a fresh request');
  if (!params.consumerReply || params.consumerReply.idlSha256 !== params.inboundProfile.consumer.idlSha256 ||
      typeof params.consumerReply.verifyEffect !== 'function') {
    throw new Error('HOLD: exact deployed consumer decoder and effect verification are required');
  }
  const { transaction, proof } = await prepareEthToVaraRelay({ ...params, deadline });
  const gasLimit = params.gearApi.blockGasLimit.toBigInt() / 100n * 95n;
  transaction.withAccount(params.signer, params.signerOptions);
  await withOriginalDeadline(deadline, () => transaction.calculateGas());
  if (transaction.gasInfo.min_limit.toBigInt() > gasLimit) throw new Error('Relay demand exceeds the source outer gas ceiling');
  transaction.withValue(0n);
  const requestPayload = transaction.extrinsic.args[1].toHex();
  const sender = decodeAddress(typeof params.signer === 'string' ? params.signer : params.signer.address);
  const signingPin = (await withOriginalDeadline(deadline, () => params.gearApi.blocks.getFinalizedHead())).toHex();
  await withOriginalDeadline(deadline, () => authenticateEffectProfile(params.gearApi, params.inboundProfile, signingPin));
  await withOriginalDeadline(deadline, () => transaction.extrinsic.signAsync(params.signer, params.signerOptions));
  if (Date.now() >= deadline) throw new Error('Original relay deadline expired before submission');
  const txHash = transaction.extrinsic.hash.toHex();
  let unsubscribe: (() => void) | undefined;
  let finished = false;
  try {
    const { blockHash, msgId } = await withOriginalDeadline(deadline, () => new Promise<{ blockHash: HexString; msgId: HexString }>((resolve, reject) => {
      transaction.extrinsic.send(({ events, status }) => {
        if (status.isInvalid || status.isDropped || status.isUsurped) {
          reject(new Error('HOLD: original admitted Gear request needs reconciliation: ' + txHash));
          return;
        }
        if (!status.isInBlock && !status.isFinalized) return;
        const failed = events.find(({ event }) => params.gearApi.events.system.ExtrinsicFailed.is(event));
        if (failed) {
          const error = params.gearApi.getExtrinsicFailedError(failed.event);
          reject(new Error(`${error.method}: ${error.docs}`));
          return;
        }
        const queued = events.filter(({ event }) => event.section === 'gear' && event.method === 'MessageQueued' &&
          event.data[1].toHex().toLowerCase() === sender.toLowerCase() &&
          event.data[2].toHex().toLowerCase() === params.historicalProxyId.toLowerCase());
        if (queued.length !== 1) {
          reject(new Error('HOLD: original Gear request inclusion has no unambiguous queued identity: ' + txHash));
          return;
        }
        resolve({ blockHash: (status.isInBlock ? status.asInBlock : status.asFinalized).toHex(),
          msgId: queued[0].event.data[0].toHex() });
      }).then((stop) => {
        if (finished) stop();
        else unsubscribe = stop;
      }).catch(reject);
    }));
    const result = await validateFinalizedEthToVaraReply({
      gearApi: params.gearApi, historicalProxyId: params.historicalProxyId, sender,
      blockHash, msgId, txHash, requestPayload, receiptRlp: proof.receiptRlp, deadline, statusCb: params.statusCb,
    });
    if (result.error) throw new Error('Finalized proxy error: ' + JSON.stringify(result.error));
    await withOriginalDeadline(deadline, () => authenticateEffectProfile(params.gearApi, params.inboundProfile, result.replyBlockHash));
    if (!result.clientReply) throw new Error('Missing original consumer reply');
    const decoded = consumerResult(result.clientReply, params.inboundProfile, params.consumerReply);
    let value: unknown;
    if (decoded.isOk && !decoded.isErr) {
      value = decoded.asOk.toJSON(); result.effectBlockHash = result.replyBlockHash; result.effectBlockNumber = result.replyBlockNumber;
      const native = params.consumerReply.nativeSettlement;
      if (native) {
        const rows = structuredClone(await withOriginalDeadline(deadline, () => native.readDeposits(result.replyBlockHash, proof)));
        if (rows.some(row => row.native || row.outcome === 'Refunded')) {
          if (!params.inboundProfile.nativeWrapper ||
              params.inboundProfile.consumer.idlSha256 !== VFT_MANAGER_IDL_SHA256 ||
              params.inboundProfile.consumer.service !== 'VftManager' || params.inboundProfile.consumer.method !== 'SubmitReceipt') {
            throw new Error('HOLD: native completion requires the approved wrapper and manager');
          }
          const wrapper = await withOriginalDeadline(deadline, () => params.gearApi.programStorage.getProgram(params.inboundProfile.nativeWrapper!.programId, result.replyBlockHash));
          if (!wrapper.state.isInitialized || !wrapper.codeId.eq(params.inboundProfile.nativeWrapper.codeId) ||
              await withOriginalDeadline(deadline, () => native.readReceiptStatus(result.replyBlockHash, proof)) !== 'Processed') {
            throw new Error('HOLD: original native receipt or wrapper is not authenticated');
          }
          const redemptions = await readNativeRedemptions(rows, result.replyBlockHash,
            (operationId, pin) => withOriginalDeadline(deadline, () => native.readRedemption(operationId, pin)));
          result.nativeSettlements = validateNativeTokenEffect(nativeReceiptIdentity(params.inboundProfile, proof, native.expectedEffect), rows, redemptions);
        }
      }
    }
    else {
      assertNativePendingReply(result.clientReply, params.inboundProfile, params.consumerReply);
      await reconcileOriginalNativeSettlement(params, proof, result, deadline);
      value = null;
    }
    if (result.effectBlockHash !== result.replyBlockHash) await withOriginalDeadline(deadline, () => authenticateEffectProfile(params.gearApi, params.inboundProfile, result.effectBlockHash!));
    await withOriginalDeadline(deadline, () => params.consumerReply.verifyEffect(value, result, proof));
    if (result.effectBlockNumber !== undefined && (await withOriginalDeadline(deadline, () => params.gearApi.blocks.getBlockHash(result.effectBlockNumber!))).toHex() !== result.effectBlockHash) {
      throw new Error('Original economic-effect finalized pin changed during readback');
    }
    if ((await withOriginalDeadline(deadline, () => params.gearApi.blocks.getBlockHash(result.replyBlockNumber))).toHex() !== result.replyBlockHash) {
      throw new Error('Original consumer finalized pin changed during effect readback');
    }
    return result;
  } catch (error) {
    const reason = error instanceof Error ? error.message : String(error);
    throw new Error(`HOLD: original Gear transaction ${txHash}: ${reason}. Reconcile the original before retrying.`);
  } finally {
    finished = true;
    unsubscribe?.();
  }
}
