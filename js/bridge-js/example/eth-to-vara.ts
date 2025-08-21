import { createPublicClient, createWalletClient, http, webSocket, zeroAddress } from 'viem';
import { privateKeyToAccount } from 'viem/accounts';
import { execSync } from 'child_process';
import { Keyring } from '@polkadot/api';
import { GearApi } from '@gear-js/api';
import { hoodi } from 'viem/chains';
import dotenv from 'dotenv';
import assert from 'assert';
import * as fs from 'fs';

import { relayEthToVara } from '../src';
import { PingClient } from './lib';

dotenv.config();

const assertEnv = (name: string) =>
  assert.notStrictEqual(process.env[name], undefined, `Missing ${name} environment variable`);

assertEnv('CHECKPOINT_CLIENT_ID');
const CHECKPOINT_CLIENT_ID = process.env.CHECKPOINT_CLIENT_ID! as `0x${string}`;
assertEnv('HISTORICAL_PROXY_ID');
const HISTORICAL_PROXY_ID = process.env.HISTORICAL_PROXY_ID! as `0x${string}`;
assertEnv('BEACON_RPC_URL');
const BEACON_RPC = process.env.BEACON_RPC_URL!;
assertEnv('ETH_RPC_URL');
const ETH_WS_RPC = process.env.ETH_RPC_URL!;
assertEnv('VARA_WS_RPC');
const VARA_WS_RPC = process.env.VARA_WS_RPC!;
assertEnv('ETH_PRIVATE_KEY');
const ETH_PRIVATE_KEY = process.env.ETH_PRIVATE_KEY! as `0x${string}`;
const PING_SERVICE_NAME = 'Ping';
const PING_METHOD_NAME = 'SubmitReceipt';
const PING_WASM_PATH = '../../target/wasm32-gear/release/ping.opt.wasm';

if (!fs.existsSync(PING_WASM_PATH)) {
  console.log(`Ping wasm wasn't found. Building the program...`);
  execSync('cargo build -p ping --release', { stdio: 'inherit' });
}

const main = async () => {
  const publicClient = createPublicClient({ transport: webSocket(ETH_WS_RPC) });
  const gearApi = await GearApi.create({ providerAddress: VARA_WS_RPC, noInitWarn: true });

  const keyring = new Keyring({ type: 'sr25519', ss58Format: 137 });
  const account = keyring.createFromUri('//Alice');

  const pingClient = new PingClient(gearApi);
  const code = fs.readFileSync(PING_WASM_PATH);

  const tx = pingClient.newCtorFromCode(code).withAccount(account).withGas('max');
  const { response } = await tx.signAndSend();

  await response();

  const pingProgramId = pingClient.programId;

  console.log(`Ping program uploaded with id: ${pingProgramId}`);

  const walletClient = createWalletClient({
    chain: hoodi,
    transport: http(),
    account: privateKeyToAccount(ETH_PRIVATE_KEY),
  });

  const ethTxHash = await walletClient.sendTransaction({
    to: zeroAddress,
    value: 0n,
  });

  console.log(`Ethereum transaction sent with hash: ${ethTxHash}. Waiting for 15 block confirmations...`);

  const receipt = await publicClient.waitForTransactionReceipt({ hash: ethTxHash, confirmations: 5 });

  if (receipt.status !== 'success') {
    throw new Error(`Ethereum transaction failed with status: ${receipt.status}`);
  }

  console.log(`Ethereum transaction confirmed. Starting relayer...`);

  const { error, ok } = await relayEthToVara(
    ethTxHash,
    BEACON_RPC,
    publicClient,
    gearApi,
    CHECKPOINT_CLIENT_ID,
    HISTORICAL_PROXY_ID,
    pingProgramId,
    PING_SERVICE_NAME,
    PING_METHOD_NAME,
    account,
    undefined,
    false,
  );

  if (error) {
    throw new Error(JSON.stringify(error));
  }

  console.log(`Done. Reply from client: ${ok}`);
};

main()
  .catch((error) => {
    console.error(error);
    process.exit(1);
  })
  .then(() => process.exit());
