import { test, expect } from 'vitest';
import { readFileSync } from 'node:fs';
import { createHash } from 'node:crypto';
import { ssz } from '@lodestar/types';
import { Trie } from '@ethereumjs/trie';
import { encode as rlpEncode } from '@ethereumjs/rlp';
import { bytesToHex, hexToBytes, keccak256, formatTransactionReceipt, type Hex, type RpcTransactionReceipt } from 'viem';
import { generateMerkleProof } from '../src/eth-to-vara/proof-composer.js';

const directory = new URL('../../../tests/src/checkpoint_light_client/chain-data/', import.meta.url);
type Capture = { path: string; sha256: string };
type Fixture = { fork: 'electra' | 'fulu'; slot: number; beacon_root: Hex; execution_hash: Hex;
  execution_number: number; selected_transaction_index: number; files: Record<string, Capture> };
const provenance = JSON.parse(readFileSync(new URL('execution-requests-provenance.json', directory), 'utf8')) as { fixtures: Fixture[] };

function capture(file: Capture) {
  const bytes = readFileSync(new URL(file.path, directory));
  expect(createHash('sha256').update(bytes).digest('hex')).toBe(file.sha256);
  return JSON.parse(bytes.toString('utf8'));
}

for (const fixture of provenance.fixtures) {
  test(`SDK genuine ${fixture.fork} nonempty requests bind Lodestar, execution RLP and receipt MPT`, async () => {
    // These are public finalized endpoint captures, not an approved bootstrap or finality bypass.
    const response = capture(fixture.files.beacon_block);
    const header = capture(fixture.files.beacon_header);
    const execution = capture(fixture.files.execution_block).result;
    const receipts = (capture(fixture.files.receipts).result as RpcTransactionReceipt[]).map(receipt => formatTransactionReceipt(receipt));
    const types = ssz[fixture.fork];
    const block = types.BeaconBlock.fromJson(response.data.message);
    expect(response.version).toBe(fixture.fork);
    expect(response.finalized).toBe(true);
    expect(response.execution_optimistic).toBe(false);
    expect(header.finalized).toBe(true);
    expect(header.data.canonical).toBe(true);
    expect(header.data.root).toBe(fixture.beacon_root);
    expect(bytesToHex(types.BeaconBlock.hashTreeRoot(block))).toBe(fixture.beacon_root);
    expect(bytesToHex(ssz.phase0.BeaconBlockHeader.hashTreeRoot(ssz.phase0.BeaconBlockHeader.fromJson(header.data.header.message)))).toBe(fixture.beacon_root);
    expect(block.slot).toBe(fixture.slot);
    expect(Object.keys(types.ExecutionPayload.fields)).toHaveLength(17);
    expect(response.data.message.body.execution_payload).not.toHaveProperty('requests_hash');
    expect(block.body.executionRequests.consolidations).toHaveLength(1);
    expect(block.body.executionRequests.deposits).toHaveLength(0);
    expect(block.body.executionRequests.withdrawals).toHaveLength(0);
    const request = block.body.executionRequests.consolidations[0];
    const requestBytes = Buffer.concat([Buffer.from([2]), request.sourceAddress, request.sourcePubkey, request.targetPubkey]);
    const requestHash = createHash('sha256').update(createHash('sha256').update(requestBytes).digest()).digest('hex');
    expect(execution.requestsHash).toBe('0x' + requestHash);
    expect(bytesToHex(block.body.executionPayload.blockHash)).toBe(fixture.execution_hash);
    expect(block.body.executionPayload.blockNumber).toBe(fixture.execution_number);
    // Complete Prague/Osaka header RLP. Quantity zero encodes as the empty byte string.
    const quantity = (value: Hex) => {
      const digits = BigInt(value).toString(16);
      return BigInt(value) === 0n ? new Uint8Array() : hexToBytes(('0x' + (digits.length % 2 ? '0' : '') + digits) as Hex);
    };
    const headerFields = [execution.parentHash, execution.sha3Uncles, execution.miner, execution.stateRoot,
      execution.transactionsRoot, execution.receiptsRoot, execution.logsBloom].map(value => hexToBytes(value));
    headerFields.push(...[execution.difficulty, execution.number, execution.gasLimit, execution.gasUsed, execution.timestamp].map(quantity),
      hexToBytes(execution.extraData), hexToBytes(execution.mixHash), hexToBytes(execution.nonce), quantity(execution.baseFeePerGas),
      hexToBytes(execution.withdrawalsRoot), quantity(execution.blobGasUsed), quantity(execution.excessBlobGas),
      hexToBytes(execution.parentBeaconBlockRoot), hexToBytes(execution.requestsHash));
    expect(keccak256(rlpEncode(headerFields))).toBe(fixture.execution_hash);
    const changedHeader = headerFields.map(field => field.slice());
    changedHeader[20][0] ^= 1;
    expect(keccak256(rlpEncode(changedHeader))).not.toBe(fixture.execution_hash);
    expect(receipts).toHaveLength(block.body.executionPayload.transactions.length);
    for (const [index, receipt] of receipts.entries()) {
      expect(receipt.transactionIndex).toBe(index);
      expect(receipt.blockHash).toBe(fixture.execution_hash);
      expect(receipt.blockNumber).toBe(BigInt(fixture.execution_number));
      expect(receipt.transactionHash).toBe(keccak256(block.body.executionPayload.transactions[index]));
      expect(execution.transactions[index]).toBe(receipt.transactionHash);
    }
    const index = fixture.selected_transaction_index;
    const proof = await generateMerkleProof(index, receipts);
    expect(bytesToHex(proof.root)).toBe(execution.receiptsRoot);
    expect(bytesToHex(proof.root)).toBe(bytesToHex(block.body.executionPayload.receiptsRoot));
    const trie = await Trie.create();
    const receiptBytes = proof.receiptRlp;
    expect(await trie.verifyProof(proof.root, rlpEncode(index), proof.proof)).toEqual(receiptBytes);
    const retained = capture(fixture.files.inclusion_proof);
    expect(retained.index).toBe(index);
    expect(retained.receipt_root).toBe(execution.receiptsRoot);
    expect(retained.transaction_root).toBe(execution.transactionsRoot);
    expect(retained.receipt_encoded).toBe(bytesToHex(receiptBytes));
    expect(retained.raw_transaction).toBe(bytesToHex(block.body.executionPayload.transactions[index]));
    expect(await trie.verifyProof(hexToBytes(retained.transaction_root), rlpEncode(index),
      retained.transaction_proof.map((node: Hex) => hexToBytes(node)))).toEqual(hexToBytes(retained.raw_transaction));
    expect(await trie.verifyProof(proof.root, rlpEncode(index),
      retained.receipt_proof.map((node: Hex) => hexToBytes(node)))).toEqual(receiptBytes);
    // A proof of this original must not authorize the same bytes at another trie position.
    const wrongPosition = await trie.verifyProof(proof.root, rlpEncode(index + 1), proof.proof).catch(() => null);
    expect(wrongPosition).not.toEqual(receiptBytes);
    const requestsChanged = types.BeaconBlock.clone(block);
    requestsChanged.body.executionRequests.consolidations = [];
    expect(bytesToHex(types.BeaconBlock.hashTreeRoot(requestsChanged))).not.toBe(fixture.beacon_root);
    const hashChanged = types.BeaconBlock.clone(block);
    hashChanged.body.executionPayload.blockHash[0] ^= 1;
    expect(bytesToHex(types.BeaconBlock.hashTreeRoot(hashChanged))).not.toBe(fixture.beacon_root);
  });
}
