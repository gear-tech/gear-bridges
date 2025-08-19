import { createPublicClient, webSocket } from 'viem';
import { execSync } from 'child_process';
import * as path from 'path';
import * as fs from 'fs';
import dotenv from 'dotenv';

dotenv.config();

const TARGET_DIR = '../../target';
const PATH_TO_BIN = path.join(TARGET_DIR, 'release/js-proof-test');

const getTxHash = async () => {
  const publicClient = createPublicClient({ transport: webSocket(process.env.ETH_RPC_URL!) });

  let block = await publicClient.getBlock({ blockTag: 'finalized' });

  if (block.transactions.length == 0) {
    while (block.transactions.length == 0) {
      block = await publicClient.getBlock({ blockNumber: block.number - 1n });
    }
  }

  return block.transactions[0];
};

export default async () => {
  if (!fs.existsSync(PATH_TO_BIN)) {
    execSync(`cargo build -p js-proof-test --release`, { stdio: 'inherit' });
  }

  const txHash = await getTxHash();

  execSync(`${PATH_TO_BIN} ${txHash}`);

  process.env['TX_HASH'] = txHash;
};
