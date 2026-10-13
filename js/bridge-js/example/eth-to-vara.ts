import assert from 'node:assert/strict';
import { readFileSync, statSync } from 'node:fs';
import { createHash } from 'node:crypto';
import { GearApi, HexString } from '@gear-js/api';
import { Keyring } from '@polkadot/api';
import { cryptoWaitReady } from '@polkadot/util-crypto';
import { createPublicClient, http } from 'viem';
import { parseInboundProofProfile, relayEthToVara } from '../src/index.js';
import { PingClient, type Delivery } from './lib.js';

// The caller records and approves this original intent before running the example.
interface Intent {
  transactionHash: HexString; applicationId: HexString; payload: HexString; deadline: number; signerPublicKey: HexString;
  profile: unknown;
}

async function main() {
  assert(process.argv.length === 3, 'Usage: example:eth-to-vara /absolute/path/original-intent.json');
  const intent: Intent = JSON.parse(readFileSync(process.argv[2], 'utf8'));
  assert(/^0x[0-9a-fA-F]{64}$/.test(intent.transactionHash), 'Original Ethereum transaction hash required');
  assert(/^0x[0-9a-fA-F]{64}$/.test(intent.applicationId) && BigInt(intent.applicationId) !== 0n, 'Nonzero application ID required');
  assert(/^0x(?:[0-9a-fA-F]{2}){0,1024}$/.test(intent.payload), 'Expected original payload required');
  assert(Number.isSafeInteger(intent.deadline) && Date.now() < intent.deadline, 'Original deadline expired or absent');
  const profile = parseInboundProofProfile(JSON.stringify(intent.profile));
  assert(profile.consumer.service === 'Ping' && profile.consumer.method === 'SubmitReceipt', 'Use the approved Ping deployment');
  for (const name of ['ETH_RPC_URL', 'VARA_WS_RPC', 'BEACON_RPC_URL', 'GEAR_SURI_FILE', 'PING_IDL_PATH']) {
    assert(process.env[name], 'Missing ' + name);
  }
  const idl = readFileSync(process.env.PING_IDL_PATH!);
  assert.equal(createHash('sha256').update(idl).digest('hex'), profile.consumer.idlSha256, 'Ping IDL differs from approval');
  // This generated registry is tied to the checked-in ABI, not an arbitrary supplied IDL.
  assert.equal(idl.toString(), readFileSync(new URL('../../../ping/ping.idl', import.meta.url), 'utf8'), 'Unsupported Ping ABI');
  const secret = statSync(process.env.GEAR_SURI_FILE!);
  assert(secret.isFile() && (secret.mode & 0o777) === 0o600, 'Signer file must be regular mode 0600');
  await cryptoWaitReady();
  const signer = new Keyring({ type: 'sr25519' }).addFromUri(readFileSync(process.env.GEAR_SURI_FILE!, 'utf8').trim());
  assert.equal('0x' + Buffer.from(signer.publicKey).toString('hex'), intent.signerPublicKey.toLowerCase(), 'Original signer mismatch');
  const ethereumPublicClient = createPublicClient({ transport: http(process.env.ETH_RPC_URL!) });
  const gearApi = await GearApi.create({ providerAddress: process.env.VARA_WS_RPC! });
  try {
    const ping = new PingClient(gearApi, profile.consumer.programId);
    const result = await relayEthToVara({ transactionHash: intent.transactionHash, beaconRpcUrl: process.env.BEACON_RPC_URL!,
      ethereumPublicClient, gearApi, historicalProxyId: profile.historicalProxyId, inboundProfile: profile,
      clientId: profile.consumer.programId, clientServiceName: 'Ping', clientMethodName: 'SubmitReceipt', signer,
      wait: true, deadline: intent.deadline, consumerReply: {
        registry: ping.registry, resultType: 'Result<Delivery, PingError>', idlSha256: profile.consumer.idlSha256,
        verifyEffect: async (value, result, proof) => {
          const expected: Delivery = { application_id: intent.applicationId.toLowerCase() as HexString, payload: intent.payload.toLowerCase() as HexString };
          assert.deepEqual(value, expected, 'Original consumer reply differs from expected delivery');
          const pin = result.effectBlockHash ?? result.replyBlockHash;
          const delivery = await ping.ping.received(BigInt(proof.proofBlock.block.slot), BigInt(proof.transactionIndex)).atBlock(pin).call();
          assert.deepEqual(delivery, expected, 'Original receipt has no exact finalized application effect');
          assert.equal(await ping.ping.payloadOf(intent.applicationId).atBlock(pin).call(), expected.payload, 'Finalized payload mismatch');
        },
      },
    });
    assert(!result.error && await result.isFinalized, 'Original relay did not finalize');
    console.log('Verified original application delivery', { txHash: result.txHash, replyBlockHash: result.replyBlockHash });
  } finally { await gearApi.disconnect(); }
}

main().catch(() => {
  console.error('HOLD: original intent not proven delivered. Reconcile it before any retry; do not replace its transaction or deadline.');
  process.exitCode = 1;
});
