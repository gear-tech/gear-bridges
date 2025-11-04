import { ByteVectorType, ContainerType, UintNumberType, ListCompositeType } from '@chainsafe/ssz';
import { MapDB, hexToBytes, bigIntToBytes, concatBytes } from '@ethereumjs/util';
import { TransactionReceipt, TransactionType } from 'viem';
import { encode as rlpEncode } from '@ethereumjs/rlp';
import { Trie } from '@ethereumjs/trie';
import { ssz } from '@lodestar/types';

import { BeaconClient, EthereumClient } from '../ethereum/index.js';
import {
  BlockGenericForBlockBody,
  BlockHeader,
  BlockInclusionProof,
  CheckpointClient,
  EthEventsClient,
  HistoricalProxyClient,
  ProofResult,
} from '../vara/index.js';
import { StatusCb } from '../util.js';

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

function txTypeToBytes(txType: TransactionType): Uint8Array {
  switch (txType) {
    case 'legacy':
      return new Uint8Array();
    case 'eip2930':
      return Uint8Array.of(0x01);
    case 'eip1559':
      return Uint8Array.of(0x02);
    case 'eip4844':
      return Uint8Array.of(0x03);
    case 'eip7702':
      return Uint8Array.of(0x04);
    default: {
      throw new Error(`Unknown tx type: ${txType}`);
    }
  }
}

export async function composeProof(
  beaconClient: BeaconClient,
  ethClient: EthereumClient,
  historicalProxyClient: HistoricalProxyClient,
  txHash: `0x${string}`,
  wait = false,
  statusCb: StatusCb = () => {},
): Promise<ProofResult> {
  statusCb(`Requesting transaction receipt`, { txHash });
  const receipt = await ethClient.getTransactionReceipt(txHash);

  const block = await ethClient.getBlockByHash(receipt.blockHash);
  const blockNumber = Number(block.number);

  statusCb(`Requesting block receipts`, { blockNumber: blockNumber.toString() });
  const receipts = await Promise.all(block.transactions.map((hash) => ethClient.getTransactionReceipt(hash)));

  const slot = await ethClient.getSlot(blockNumber);

  statusCb(`Generating merkle proof`, { transactionIndex: receipt.transactionIndex.toString() });

  const { proof, receiptRlp } = await generateMerkleProof(receipt.transactionIndex, receipts);

  statusCb(`Looking for checkpoint client`, { slot: slot.toString() });
  const endpoint = await historicalProxyClient.historicalProxy.endpointFor(slot).call();
  if ('err' in endpoint) {
    throw new Error(`Failed to get endpoint for slot ${slot}. Error: ${JSON.stringify(endpoint.err)}`);
  }
  const ethEventsClient = new EthEventsClient(historicalProxyClient.api, endpoint.ok);

  const checkpointAddr = await ethEventsClient.ethereumEventClient.checkpointLightClientAddress().call();
  const checkpointClient = new CheckpointClient(historicalProxyClient.api, checkpointAddr);

  statusCb(`Building inclusion proof`, { slot: slot.toString(), checkpointClient: checkpointAddr });
  const proofBlock = await buildInclusionProof(beaconClient, checkpointClient, slot, wait, statusCb);

  return {
    proofBlock,
    proof,
    transactionIndex: receipt.transactionIndex,
    receiptRlp,
  };
}

async function buildInclusionProof(
  beaconClient: BeaconClient,
  checkpointClient: CheckpointClient,
  slot: number,
  wait = false,
  statusCb: StatusCb = () => {},
): Promise<BlockInclusionProof> {
  const beaconBlock = await beaconClient.getBlock(slot);

  const body = ssz.electra.BeaconBlockBody.fromJson(beaconBlock.body);

  const block: BlockGenericForBlockBody = {
    slot,
    proposerIndex: BigInt(beaconBlock.proposer_index),
    parentRoot: hexToBytes(beaconBlock.parent_root),
    stateRoot: hexToBytes(beaconBlock.state_root),
    body: {
      randaoReveal: BytesFixed96.hashTreeRoot(body.randaoReveal),
      eth1Data: Eth1Data.hashTreeRoot(body.eth1Data),
      graffiti: body.graffiti,
      proposerSlashings: ProposerSlashings.hashTreeRoot(body.proposerSlashings),
      attesterSlashings: AttesterSlashings.hashTreeRoot(body.attesterSlashings),
      attestations: Attestations.hashTreeRoot(body.attestations),
      deposits: Deposits.hashTreeRoot(body.deposits),
      voluntaryExits: VoluntaryExits.hashTreeRoot(body.voluntaryExits),
      syncAggregate: ssz.electra.SyncAggregate.hashTreeRoot(body.syncAggregate),
      executionPayload: {
        ...body.executionPayload,
        logsBloom: BytesFixed256.hashTreeRoot(body.executionPayload.logsBloom),
        transactions: ssz.electra.Transactions.hashTreeRoot(body.executionPayload.transactions),
        withdrawals: ssz.electra.Withdrawals.hashTreeRoot(body.executionPayload.withdrawals),
      },
      blsToExecutionChanges: BlsToExecutionChanges.hashTreeRoot(body.blsToExecutionChanges),
      blobKzgCommitments: ssz.electra.BlobKzgCommitments.hashTreeRoot(body.blobKzgCommitments),
      executionRequests: ssz.electra.ExecutionRequests.hashTreeRoot(body.executionRequests),
    },
  };

  statusCb(`Requesting slot from Checkpoint Client program`, { slot: slot.toString() });
  const checkpointSlot = await checkpointClient.serviceCheckpointFor.get(slot, wait, statusCb);

  if (checkpointSlot[0] === slot) {
    return { block, headers: [] };
  }

  const beaconHeaders = await beaconClient.requestHeaders(slot + 1, checkpointSlot[0]);

  const headers: BlockHeader[] = beaconHeaders.map(({ header: { message } }) => ({
    slot: message.slot,
    proposerIndex: message.proposer_index,
    parentRoot: hexToBytes(message.parent_root),
    stateRoot: hexToBytes(message.state_root),
    bodyRoot: hexToBytes(message.body_root),
  }));

  return { block, headers };
}

function rlpEncodeReceipt(receipt: TransactionReceipt): Uint8Array {
  // https://eips.ethereum.org/EIPS/eip-2718#receipts
  const status = receipt.status === 'success' ? Uint8Array.from([1]) : Uint8Array.from([]);
  const cumulativeGasUsed = bigIntToBytes(receipt.cumulativeGasUsed);
  const bloom = hexToBytes(receipt.logsBloom);

  const logs = receipt.logs.map((log) => {
    const address = hexToBytes(log.address);
    const data = hexToBytes(log.data);
    return [address, log.topics.map((topic) => hexToBytes(topic)), data];
  });

  const txType = txTypeToBytes(receipt.type);
  const data = [status, cumulativeGasUsed, bloom, logs];
  const innerReceipt = rlpEncode(data);

  return concatBytes(txType, innerReceipt);
}

const rlpEncodeTransactionIndex = (index: number): Uint8Array => rlpEncode(index);

const rlpEncodeIndexAndReceipt = (
  index: number,
  receipt: TransactionReceipt,
): [encodedIndex: Uint8Array, encodedReceipt: Uint8Array] => [
  rlpEncodeTransactionIndex(index),
  rlpEncodeReceipt(receipt),
];

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
    receiptRlp: rlpEncode(receipt),
  };
}
