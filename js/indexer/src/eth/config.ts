import { getEnv } from '../common/index.js';

let apiPath = getEnv('ETH_API_PATH', './api/ethereum');

apiPath = apiPath.endsWith('/') ? apiPath.slice(0, -1) : apiPath;

export const config = {
  archiveUrl: getEnv('ETH_ARCHIVE_URL', 'https://v2.archive.subsquid.io/network/ethereum-holesky'),
  rpcUrl: getEnv('ETH_RPC_URL'),
  rateLimit: Number(getEnv('ETH_RATE_LIMIT', '10')),
  erc20Manager: getEnv('ETH_ERC20_MANAGER').toLowerCase(),
  msgQ: getEnv('ETH_MSQ_QUEUE').toLowerCase(),
  bridgingPayment: getEnv('ETH_BRIDGING_PAYMENT').toLowerCase(),
  fromBlock: Number(getEnv('ETH_FROM_BLOCK', '2636000')),
  apiPath,
};
