import { ByteVectorType, ContainerType, UintNumberType, ListCompositeType } from '@chainsafe/ssz';
import { MapDB, hexToBytes, bigIntToBytes, bytesToHex } from '@ethereumjs/util';
import { Trie, bytesToNibbles } from '@ethereumjs/trie';
import { TransactionReceipt, keccak256 } from 'viem';
import { encode } from '@ethereumjs/rlp';
import { ssz } from '@lodestar/types';

import { EthereumClient } from './ethereum-client';
import { BeaconClient } from './beacon-client';

const MAX_ATTESTER_SLASHINGS = 1;
const MAX_PROPOSER_SLASHINGS = 16;
const MAX_ATTESTATIONS = 8;
const MAX_DEPOSITS = 16;
const MAX_VOLUNTARY_EXITS = 16;
const MAX_BLS_TO_EXECUTION_CHANGES = 16;

const BytesFixed96 = new ByteVectorType(96);
const BytesFixed256 = new ByteVectorType(256);
const Eth1Data = new ContainerType({
  depositRoot: new ByteVectorType(32),
  depositCount: new UintNumberType(4),
  blockHash: new ByteVectorType(32),
});
const ProposerSlashings = new ListCompositeType(ssz.electra.ProposerSlashing, MAX_PROPOSER_SLASHINGS);
const AttesterSlashings = new ListCompositeType(ssz.electra.AttesterSlashing, MAX_ATTESTER_SLASHINGS);
const Attestations = new ListCompositeType(ssz.electra.Attestation, MAX_ATTESTATIONS);
const Deposits = new ListCompositeType(ssz.electra.Deposit, MAX_DEPOSITS);
const VoluntaryExits = new ListCompositeType(ssz.electra.SignedVoluntaryExit, MAX_VOLUNTARY_EXITS);
const BlsToExecutionChanges = new ListCompositeType(
  ssz.electra.SignedBLSToExecutionChange,
  MAX_BLS_TO_EXECUTION_CHANGES,
);

export async function composeProof(beaconClient: BeaconClient, ethClient: EthereumClient, txHash: `0x${string}`) {
  const _receipt = await ethClient.getTransactionReceipt(txHash);

  const block = await ethClient.getBlockByHash(_receipt.blockHash);

  const blockNumber = Number(block.number);

  const receipts = await Promise.all(block.transactions.map((hash) => ethClient.getTransactionReceipt(hash)));

  const slot = await ethClient.getSlot(blockNumber);

  const { proof, receipt } = await generateMerkleProof(_receipt.transactionIndex, receipts);

  const proofBlock = await buildInclusionProof(beaconClient, slot);

  return {
    proof_block: proofBlock,
    proof: bytesToHex(proof[0]),
    transaction_index: _receipt.transactionIndex,
    receipt_rlp: bytesToHex(receipt),
  };
}

async function buildInclusionProof(beaconClient: BeaconClient, slot: number) {
  const beaconBlock = await beaconClient.getBlock(slot);

  const body = ssz.electra.BeaconBlockBody.fromJson(beaconBlock.body);

  const block = {
    slot,
    proposer_index: beaconBlock.proposer_index,
    parent_root: beaconBlock.parent_root,
    state_root: beaconBlock.state_root,
    body: {
      randao_reveal: BytesFixed96.hashTreeRoot(body.randaoReveal),
      eth1_data: Eth1Data.hashTreeRoot(body.eth1Data),
      graffity: body.graffiti,
      proposer_slashings: ProposerSlashings.hashTreeRoot(body.proposerSlashings),
      attester_slashings: AttesterSlashings.hashTreeRoot(body.attesterSlashings),
      attestations: Attestations.hashTreeRoot(body.attestations),
      deposits: Deposits.hashTreeRoot(body.deposits),
      voluntary_exits: VoluntaryExits.hashTreeRoot(body.voluntaryExits),
      sync_aggregate: ssz.electra.SyncAggregate.hashTreeRoot(body.syncAggregate),
      execution_payload: {
        ...body.executionPayload,
        logsBloom: BytesFixed256.hashTreeRoot(body.executionPayload.logsBloom),
        transactions: ssz.electra.Transactions.hashTreeRoot(body.executionPayload.transactions),
        withdrawals: ssz.electra.Withdrawals.hashTreeRoot(body.executionPayload.withdrawals),
      },
      bls_to_execution_changes: BlsToExecutionChanges.hashTreeRoot(body.blsToExecutionChanges),
      blob_kzg_commitments: ssz.electra.BlobKzgCommitments.hashTreeRoot(body.blobKzgCommitments),
      execution_requests: ssz.electra.ExecutionRequests.hashTreeRoot(body.executionRequests),
    },
  };

  const header = {
    slot,
    proposer_index: beaconBlock.proposer_index,
    parent_root: beaconBlock.parent_root,
    state_root: beaconBlock.state_root,
    body_root: ssz.electra.BeaconBlockBody.hashTreeRoot(body),
  };

  return { block, headers: [header] };
}

function rlpEncodeReceipt(receipt: TransactionReceipt): Uint8Array {
  // LegacyReceipt is rlp([status, cumulativeGasUsed, logsBloom, logs])
  // https://eips.ethereum.org/EIPS/eip-2718#receipts
  const status = receipt.status === 'success' ? '0x1' : '0x0';
  const cumulativeGasUsed = bigIntToBytes(receipt.cumulativeGasUsed);
  const logsBloom = hexToBytes(receipt.logsBloom);
  const logs = receipt.logs.map((log) => [
    hexToBytes(log.address),
    log.topics.map((topic) => hexToBytes(topic)),
    hexToBytes(log.data),
  ]);

  return encode([status, cumulativeGasUsed, logsBloom, logs]);
}

function rlpEncodeTransactionIndex(index: number): Uint8Array {
  const rlp_encoded = encode(index);
  const hash = hexToBytes(keccak256(rlp_encoded));
  return bytesToNibbles(hash);
}

function rlpEncodeIndexAndReceipt(
  index: number,
  receipt: TransactionReceipt,
): [encodedIndex: Uint8Array, encodedReceipt: Uint8Array] {
  const encodedIndex = rlpEncodeTransactionIndex(index);
  const encodedReceipt = rlpEncodeReceipt(receipt);
  return [encodedIndex, encodedReceipt];
}

async function generateMerkleProof(txIndex: number, receipts: TransactionReceipt[]) {
  const targetReceipt = receipts.find((receipt) => receipt.transactionIndex === txIndex);

  if (!targetReceipt) {
    throw new Error(`Transaction receipt not found for index ${txIndex}`);
  }

  const trie = await Trie.create({
    db: new MapDB(),
  });

  for (const receipt of receipts) {
    const [encodedIndex, encodedReceipt] = rlpEncodeIndexAndReceipt(receipt.transactionIndex, receipt);
    await trie.put(encodedIndex, encodedReceipt);
  }

  const targetEncodedIndex = rlpEncodeTransactionIndex(txIndex);

  const [proof, receipt] = await Promise.all([trie.createProof(targetEncodedIndex), trie.get(targetEncodedIndex)]);

  if (!receipt) {
    throw new Error(`Value not found for index ${txIndex}`);
  }

  return {
    proof,
    receipt,
  };
}
