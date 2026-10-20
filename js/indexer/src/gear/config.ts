import { getEnv } from '../common/index.js';

let apiPath = getEnv('GEAR_API_PATH', './api/gear');

apiPath = apiPath.endsWith('/') ? apiPath.slice(0, -1) : apiPath;
const rpcOnly = getEnv('GEAR_RPC_ONLY', 'false') === 'true';
const retainedCutoverBlock = process.env.GEAR_RETAINED_CUTOVER_FROM_BLOCK;
if (retainedCutoverBlock !== undefined &&
    (!/^\d+$/.test(retainedCutoverBlock) || !Number.isSafeInteger(Number(retainedCutoverBlock)))) {
  throw new Error('Invalid GEAR_RETAINED_CUTOVER_FROM_BLOCK: expected a nonnegative safe integer');
}

export const config = {
  rpcOnly,
  retainedCutoverFromBlock: retainedCutoverBlock === undefined ? undefined : Number(retainedCutoverBlock),
  archiveUrl: getEnv('GEAR_ARCHIVE_URL', 'https://v2.archive.subsquid.io/network/vara-testnet'),
  rpcUrl: getEnv('GEAR_RPC_URL', 'wss://testnet.vara.network'),
  vftManager: getEnv('GEAR_VFT_MANAGER'),
  historicalProxy: getEnv('GEAR_HISTORICAL_PROXY'),
  bridgingPayment: getEnv('GEAR_BRIDGING_PAYMENT'),
  checkpointClient: getEnv('GEAR_CHECKPOINT_CLIENT'),
  fromBlock: Number(getEnv('GEAR_FROM_BLOCK', '11000000')),
  ethRpcUrl: getEnv('ETH_HTTP_RPC_URL'),
  rateLimit: Number(getEnv('GEAR_RATE_LIMIT', '100')),
  apiPath,
  apiKey: rpcOnly ? undefined : getEnv('GEAR_SQUID_API_KEY'),
};
