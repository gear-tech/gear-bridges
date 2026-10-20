import assert from 'node:assert/strict';
import { readFileSync, statSync } from 'node:fs';
import { GearApi, HexString } from '@gear-js/api';
import { blake2AsHex } from '@polkadot/util-crypto';
import type { Bytes } from '@polkadot/types';
import { bytesToHex, createPublicClient, createWalletClient, http, keccak256, parseAbi, parseEventLogs } from 'viem';
import { privateKeyToAccount } from 'viem/accounts';
import { prepareVaraToEthRelay } from '../src/vara-to-eth/relayer.js';
import { MessageQueueClient, type FinalizedMessageProcessResult } from '../src/ethereum/message-queue.js';
import { withOriginalDeadline } from '../src/util.js';
import { relayFailure } from './relay-failure.js';

interface Intent {
  nonce: string; blockNumber: string; applicationId: HexString; payload: HexString; deadline: number;
  ethereumChainId: string; ethereumGenesisHash: HexString; sourceGenesisHash: HexString; sourceRuntimeCodeHash: HexString;
  sourceProgramId: HexString; sourceCodeId: HexString; messageQueueAddress: HexString; queueCodeHash: HexString;
  handlerAddress: HexString; handlerCodeHash: HexString; ethereumSender: HexString;
}
const handlerAbi = parseAbi([
  'function queue() view returns (address)', 'function expectedVaraSource() view returns (bytes32)',
  'function received(bytes32) view returns (bool)', 'function payloadOf(bytes32) view returns (bytes)',
  'event MessageHandled(bytes32 indexed source, bytes32 indexed applicationId, bytes payload)',
]);

const original: { ethereumTransactionHash?: HexString; nonce?: string; sourceBlockNumber?: string } = {};

async function main() {
  assert(process.argv.length === 3, 'Usage: example:vara-to-eth /absolute/path/original-intent.json');
  const intent: Intent = JSON.parse(readFileSync(process.argv[2], 'utf8'));
  assert(/^[0-9]+$/.test(intent.nonce) && /^[0-9]+$/.test(intent.blockNumber), 'Original decimal nonce/block required; nonce zero is valid');
  original.nonce = intent.nonce;
  original.sourceBlockNumber = intent.blockNumber;
  assert(/^0x[0-9a-fA-F]{64}$/.test(intent.applicationId) && BigInt(intent.applicationId) !== 0n, 'Nonzero application ID required');
  assert(/^0x(?:[0-9a-fA-F]{2}){0,1024}$/.test(intent.payload), 'Expected original payload required');
  assert(Number.isSafeInteger(intent.deadline) && Date.now() < intent.deadline, 'Original deadline expired or absent');
  assert(/^[0-9]+$/.test(intent.ethereumChainId) && BigInt(intent.ethereumChainId) > 0n, 'Approved Ethereum chain ID required');
  for (const value of [intent.ethereumGenesisHash, intent.sourceGenesisHash, intent.sourceRuntimeCodeHash,
    intent.sourceProgramId, intent.sourceCodeId, intent.queueCodeHash, intent.handlerCodeHash]) {
    assert(/^0x[0-9a-fA-F]{64}$/.test(value) && BigInt(value) !== 0n, 'Nonzero approved deployment hash required');
  }
  for (const value of [intent.messageQueueAddress, intent.handlerAddress, intent.ethereumSender]) {
    assert(/^0x[0-9a-fA-F]{40}$/.test(value) && BigInt(value) !== 0n, 'Nonzero approved deployment address required');
  }
  for (const name of ['ETH_RPC_URL', 'VARA_WS_RPC', 'ETH_KEY_FILE']) assert(process.env[name], 'Missing ' + name);
  const secret = statSync(process.env.ETH_KEY_FILE!);
  assert(secret.isFile() && (secret.mode & 0o777) === 0o600, 'Signer file must be regular mode 0600');
  const account = privateKeyToAccount(readFileSync(process.env.ETH_KEY_FILE!, 'utf8').trim() as HexString);
  assert.equal(account.address.toLowerCase(), intent.ethereumSender.toLowerCase(), 'Original signer mismatch');
  const ethereumPublicClient = createPublicClient({ transport: http(process.env.ETH_RPC_URL!) });
  const ethereumWalletClient = createWalletClient({ account, transport: http(process.env.ETH_RPC_URL!) });
  const gearApi = await GearApi.create({ providerAddress: process.env.VARA_WS_RPC! });
  const read = <T>(operation: () => Promise<T>) => withOriginalDeadline(intent.deadline, operation);
  try {
    assert.equal(BigInt(await read(() => ethereumPublicClient.getChainId())), BigInt(intent.ethereumChainId), 'Wrong Ethereum chain');
    assert.equal((await read(() => ethereumPublicClient.getBlock({ blockNumber: 0n }))).hash, intent.ethereumGenesisHash, 'Wrong Ethereum genesis');
    assert.equal(gearApi.genesisHash.toHex(), intent.sourceGenesisHash, 'Wrong source genesis');
    const pin = (await read(() => gearApi.blocks.getFinalizedHead())).toHex();
    const runtime = await read(() => gearApi.rpc.state.getStorage<Bytes>(':code', pin));
    assert(!runtime.isEmpty && blake2AsHex(runtime.toU8a(true), 256) === intent.sourceRuntimeCodeHash, 'Wrong finalized source runtime');
    const program = await read(() => gearApi.programStorage.getProgram(intent.sourceProgramId, pin));
    assert(program.state.isInitialized && program.codeId.eq(intent.sourceCodeId), 'Wrong finalized source program');
    const finalized = await read(() => ethereumPublicClient.getBlock({ blockTag: 'finalized' }));
    for (const [address, expected] of [[intent.messageQueueAddress, intent.queueCodeHash], [intent.handlerAddress, intent.handlerCodeHash]]) {
      const code = await read(() => ethereumPublicClient.getCode({ address, blockNumber: finalized.number }));
      assert(code && code !== '0x' && keccak256(code) === expected, 'Wrong approved Ethereum deployment code');
    }
    assert.equal((await read(() => ethereumPublicClient.readContract({ address: intent.handlerAddress, abi: handlerAbi, functionName: 'queue', blockNumber: finalized.number }))).toLowerCase(), intent.messageQueueAddress.toLowerCase());
    assert.equal((await read(() => ethereumPublicClient.readContract({ address: intent.handlerAddress, abi: handlerAbi, functionName: 'expectedVaraSource', blockNumber: finalized.number }))).toLowerCase(), intent.sourceProgramId.toLowerCase());
    const prepared = await prepareVaraToEthRelay({ nonce: BigInt(intent.nonce), blockNumber: BigInt(intent.blockNumber),
      ethereumPublicClient, gearApi, messageQueueAddress: intent.messageQueueAddress, wait: true, deadline: intent.deadline });
    assert.equal(bytesToHex(prepared.message.source), intent.sourceProgramId.toLowerCase(), 'Original source differs');
    assert.equal(bytesToHex(prepared.message.destination), intent.handlerAddress.toLowerCase(), 'Original destination differs');
    assert.equal(bytesToHex(prepared.message.payload), (intent.applicationId + intent.payload.slice(2)).toLowerCase(), 'Original packed message differs');
    const result = await new MessageQueueClient(intent.messageQueueAddress, ethereumPublicClient, ethereumWalletClient, account)
      .processMessage(prepared.blockNumber, prepared.message, prepared.proof, { kind: 'application' }, undefined, intent.deadline, { wait: true });
    original.ethereumTransactionHash = result.transactionHash;
    if (!result.success) throw new Error(result.error);
    const evidence = result as FinalizedMessageProcessResult;
    const receipt = await read(() => ethereumPublicClient.getTransactionReceipt({ hash: result.transactionHash }));
    assert.equal(receipt.blockHash, evidence.receiptBlockHash, 'Original receipt pin changed');
    const events = parseEventLogs({ abi: handlerAbi, eventName: 'MessageHandled', strict: true,
      logs: receipt.logs.filter(log => log.address.toLowerCase() === intent.handlerAddress.toLowerCase()) });
    assert.equal(events.length, 1, 'Exactly one same-receipt application effect required');
    assert(!events[0].removed && events[0].transactionHash === receipt.transactionHash &&
      events[0].blockHash === receipt.blockHash && events[0].blockNumber === receipt.blockNumber &&
      events[0].transactionIndex === receipt.transactionIndex, 'Application effect is not in the original finalized receipt');
    assert.deepEqual(events[0].args, { source: intent.sourceProgramId.toLowerCase(), applicationId: intent.applicationId.toLowerCase(), payload: intent.payload.toLowerCase() });
    assert(await read(() => ethereumPublicClient.readContract({ address: intent.handlerAddress, abi: handlerAbi, functionName: 'received', args: [intent.applicationId], blockNumber: receipt.blockNumber })), 'Application delivery absent');
    assert.equal(await read(() => ethereumPublicClient.readContract({ address: intent.handlerAddress, abi: handlerAbi, functionName: 'payloadOf', args: [intent.applicationId], blockNumber: receipt.blockNumber })), intent.payload.toLowerCase(), 'Finalized payload mismatch');
    assert.equal((await read(() => ethereumPublicClient.getBlock({ blockNumber: receipt.blockNumber }))).hash, receipt.blockHash, 'Finalized effect pin changed');
    console.log('Verified original application delivery', { txHash: result.transactionHash, receiptBlockHash: receipt.blockHash });
  } finally { await gearApi.disconnect(); }
}

main().catch((error) => {
  console.error('HOLD: original intent not proven delivered. Reconcile it before any retry; do not replace its transaction or deadline.',
    relayFailure(error, original));
  process.exitCode = 1;
});
