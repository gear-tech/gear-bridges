import { getEnv } from '../common';

export const config = {
  archiveUrl: getEnv('GEAR_ARCHIVE_URL', 'https://v2.archive.subsquid.io/network/vara-testnet'),
  rpcUrl: getEnv('GEAR_RPC_URL', 'https://testnet.vara.network'),
  vftManager: getEnv('GEAR_VFT_MANAGER'),
  erc20Relay: getEnv('GEAR_ERC20_RELAY'),
  historicalProxy: getEnv('GEAR_HISTORICAL_PROXY'),
  fromBlock: Number(getEnv('GEAR_FROM_BLOCK', '11000000')),
};
