import { getEnv } from '../common/index.js';

let apiPath = getEnv('GEAR_API_PATH', './api/gear');

apiPath = apiPath.endsWith('/') ? apiPath.slice(0, -1) : apiPath;

export const config = {
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
};
