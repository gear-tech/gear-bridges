import { getEnv } from '../common';

export const config = {
  archiveUrl: getEnv('ETH_ARCHIVE_URL', 'https://v2.archive.subsquid.io/network/ethereum-holesky'),
  rpcUrl: getEnv('ETH_RPC_URL', 'https://eth-holesky-beacon.public.blastapi.io'),
  erc20Manager: getEnv('ETH_ERC20_MANAGER').toLowerCase(),
  msgQ: getEnv('ETH_MSQ_QUEUE').toLowerCase(),
  fromBlock: Number(getEnv('ETH_FROM_BLOCK', '2636000')),
};
