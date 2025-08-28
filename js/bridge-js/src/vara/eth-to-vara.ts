import { TypeRegistry } from '@polkadot/types';
import { compactAddLength, compactToU8a } from '@polkadot/util';
import { bytesToHex, concatBytes } from '@ethereumjs/util';
import {
  HistoricalProxyTypes,
  ProofResult,
  BlockInclusionProof,
  BlockGenericForBlockBody,
  BlockHeader,
  BlockBody,
  ExecutionPayload,
  U64,
  U256,
} from './historical-proxy-types';

const registry = new TypeRegistry();
registry.setKnownTypes({ types: HistoricalProxyTypes });
registry.register(HistoricalProxyTypes);

export function encodeEthToVaraEvent(payload: ProofResult): `0x${string}` {
  const proofBlock = encodeProofBlock(payload.proofBlock);
  const proof = encodeProof(payload.proof);
  const transactionIndex = encodeTransactionIndex(payload.transactionIndex);
  const receiptRlp = encodeReceiptRlp(payload.receiptRlp);
  return bytesToHex(concatBytes(proofBlock, proof, transactionIndex, receiptRlp));
}

function encodeProofBlock(payload: BlockInclusionProof): Uint8Array {
  return concatBytes(encodeBlockInclusionProofBlock(payload.block), encodeBlockInclusionProofHeaders(payload.headers));
}

function encodeProof(payload: Uint8Array[]): Uint8Array {
  const proofs = [];
  for (const item of payload) {
    proofs.push(compactAddLength(item));
  }

  const length = compactToU8a(proofs.length);

  return concatBytes(length, ...proofs);
}

function encodeTransactionIndex(payload: any): Uint8Array {
  return registry.createType('u64', payload).toU8a();
}

function encodeReceiptRlp(payload: Uint8Array): Uint8Array {
  return compactAddLength(payload);
}

function encodeBlockInclusionProofBlock(data: BlockGenericForBlockBody): Uint8Array {
  return concatBytes(
    encodeU64(data.slot),
    encodeU64(data.proposerIndex),
    data.parentRoot,
    data.stateRoot,
    encodeBlockBody(data.body),
  );
}

function encodeBlockBody(data: BlockBody): Uint8Array {
  return concatBytes(
    data.randaoReveal,
    data.eth1Data,
    data.graffiti,
    data.proposerSlashings,
    data.attesterSlashings,
    data.attestations,
    data.deposits,
    data.voluntaryExits,
    data.syncAggregate,
    encodeExecutionPayload(data.executionPayload),
    data.blsToExecutionChanges,
    data.blobKzgCommitments,
    data.executionRequests,
  );
}

function encodeExecutionPayload(data: ExecutionPayload): Uint8Array {
  return concatBytes(
    data.parentHash,
    data.feeRecipient,
    data.stateRoot,
    data.receiptsRoot,
    data.logsBloom,
    data.prevRandao,
    encodeU64(data.blockNumber),
    encodeU64(data.gasLimit),
    encodeU64(data.gasUsed),
    encodeU64(data.timestamp),
    compactAddLength(data.extraData),
    encodeU256(data.baseFeePerGas),
    data.blockHash,
    data.transactions,
    data.withdrawals,
    encodeU64(data.blobGasUsed),
    encodeU64(data.excessBlobGas),
  );
}

function encodeBlockInclusionProofHeaders(data: BlockHeader[]): Uint8Array {
  return registry.createType('Vec<BlockHeader>', data).toU8a();
}

function encodeU64(value: U64): Uint8Array {
  return registry.createType('u64', value).toU8a();
}

function encodeU256(value: U256): Uint8Array {
  return registry.createType('u256', value).toU8a();
}
