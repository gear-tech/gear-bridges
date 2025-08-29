import { bytesToHex, createPublicClient, createWalletClient, webSocket } from 'viem';
import { GearApi, HexString, MessageQueued } from '@gear-js/api';
import { concatBytes, hexToBytes } from '@ethereumjs/util';
import { privateKeyToAccount } from 'viem/accounts';
import { compactAddLength } from '@polkadot/util';
import { Keyring } from '@polkadot/api';
import dotenv from 'dotenv';
import assert from 'assert';

import { decodeEthBridgeMessageResponse, relayVaraToEth, waitForMerkleRootAppearedInMessageQueue } from '../src';

dotenv.config({ quiet: true });

const assertEnv = (name: string) =>
  assert.notStrictEqual(process.env[name], undefined, `Missing ${name} environment variable`);

assertEnv('MESSAGE_QUEUE_ADDRESS');
const MESSAGE_QUEUE_ADDRESS = process.env.MESSAGE_QUEUE_ADDRESS! as HexString;
assertEnv('ETH_RPC_URL');
const ETH_WS_RPC = process.env.ETH_RPC_URL!;
assertEnv('VARA_WS_RPC');
const VARA_WS_RPC = process.env.VARA_WS_RPC!;
assertEnv('ETH_PRIVATE_KEY');
const ETH_PRIVATE_KEY = process.env.ETH_PRIVATE_KEY! as HexString;
const GEAR_BRIDGE_BUILTIN = '0xf2816ced0b15749595392d3a18b5a2363d6fefe5b3b6153739f218151b7acdbf';
const DESTINATION = '0x187453bD463773a3af896D4bFe7A1168D206AFAB';

const [nonceArg, blockNumberArg] = process.argv.slice(2).map((arg) => {
  const match = arg.match(/^--(nonce|bn)=(\d+)$/);
  return match ? parseInt(match[2]) : undefined;
});

const main = async () => {
  const ethTransport = webSocket(ETH_WS_RPC);
  const publicClient = createPublicClient({ transport: ethTransport });
  const gearApi = await GearApi.create({ providerAddress: VARA_WS_RPC, noInitWarn: true });
  const walletClient = createWalletClient({ transport: ethTransport });
  const ethAccount = privateKeyToAccount(ETH_PRIVATE_KEY);

  if (nonceArg && blockNumberArg) {
    return await relayVaraToEth(
      BigInt(nonceArg),
      BigInt(blockNumberArg),
      publicClient,
      walletClient,
      ethAccount,
      gearApi,
      MESSAGE_QUEUE_ADDRESS,
      (status, details) => {
        console.log(`[relayEthToVara]: ${status}`, details);
      },
    );
  }

  const keyring = new Keyring({ type: 'sr25519', ss58Format: 137 });
  const varaAccount = keyring.createFromUri('//Alice');

  const payload = compactAddLength(gearApi.createType('String', 'ping').toU8a());
  const destination = hexToBytes(DESTINATION);

  const ethBridgePayload = concatBytes(Uint8Array.from([0]), destination, payload);

  console.log(`Sending message ${bytesToHex(ethBridgePayload)} to bridge builtin`);

  const tx = gearApi.message.send({
    destination: GEAR_BRIDGE_BUILTIN,
    payload: ethBridgePayload,
    gasLimit: gearApi.blockGasLimit,
  });

  const [msgId, blockHash] = await new Promise<[HexString, HexString]>((resolve, reject) =>
    tx.signAndSend(varaAccount, ({ status, events }) => {
      if (status.isInBlock) {
        const mqEvent = events.find(({ event: { method } }) => method === 'MessageQueued')?.event as MessageQueued;

        if (!mqEvent) reject(`MessageQueued event not found`);
        else resolve([mqEvent.data.id.toHex(), status.asInBlock.toHex()]);
      }
    }),
  );

  const msgBn = await gearApi.blocks.getBlockNumber(blockHash);

  console.log(`Message sent in block ${msgBn.toNumber()} with id ${msgId}`);

  const ethBridgeMessages = new Map<string, { nonce: bigint; blockhash: HexString }>();

  const unsub = await gearApi.query.system.events(async (events) => {
    if (!events.createdAtHash) {
      return;
    }

    const _events = events.filter(({ event }) => event.section === 'gearEthBridge' && event.method === 'MessageQueued');
    if (_events.length === 0) {
      return;
    }

    const blockhash = events.createdAtHash.toHex();
    const bn = await gearApi.blocks.getBlockNumber(blockhash);

    for (const { event } of _events as any) {
      const hash = event.data[1].toHex();
      const nonce = event.data[0]['nonce'].toBigInt();
      ethBridgeMessages.set(hash, { nonce, blockhash });
      console.log(`Received new eth bridge message ${hash} with nonce ${nonce} in block ${bn.toNumber()}`);
    }
  });

  const reply = await gearApi.message.getReplyEvent(GEAR_BRIDGE_BUILTIN, msgId, blockHash);

  const { nonce, hash, nonceLe } = decodeEthBridgeMessageResponse(reply.data.message.payload.toU8a());

  console.log(`Got reply with nonce ${nonce} (${nonceLe}) and hash ${hash}`);

  let ethBridgeMessage: { nonce: bigint; blockhash: HexString } | undefined = undefined;

  while (!ethBridgeMessage) {
    if (!ethBridgeMessages.has(hash)) {
      console.log(`Waiting for eth bridge message ${hash}`);
    } else {
      ethBridgeMessage = ethBridgeMessages.get(hash);
    }
  }

  unsub();

  const blockNumber = await gearApi.blocks.getBlockNumber(ethBridgeMessage.blockhash);

  await waitForMerkleRootAppearedInMessageQueue(blockNumber.toBigInt(), publicClient, MESSAGE_QUEUE_ADDRESS);

  await relayVaraToEth(
    ethBridgeMessage.nonce,
    blockNumber.toBigInt(),
    publicClient,
    walletClient,
    ethAccount,
    gearApi,
    MESSAGE_QUEUE_ADDRESS,
    (status, details) => {
      console.log(`[relayEthToVara]: ${status}`, details);
    },
  );
};

main()
  .catch((error) => {
    console.error(error);
    process.exit(1);
  })
  .then(() => process.exit());
