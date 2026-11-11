import { getEnv } from '../common/env.js';

const requiredNetworkEnv = (key: string): string => {
  const value = process.env[key]?.trim();

  if (!value) {
    throw new Error(
      `Missing network configuration: ${key}. Set RPC, archive, start block and bridge program IDs for the same deployment. See js/indexer/README.md.`,
    );
  }

  return value;
};

const networkUrl = (key: string, protocols: string[]): string => {
  const value = requiredNetworkEnv(key);
  let url: URL;

  try {
    url = new URL(value);
  } catch {
    throw new Error(`Invalid network configuration: ${key} must be a URL.`);
  }

  if (!protocols.includes(url.protocol)) {
    throw new Error(`Invalid network configuration: ${key} must use ${protocols.join(' or ')}.`);
  }

  return value;
};

const programId = (key: string): string => {
  const value = requiredNetworkEnv(key);

  if (!/^0x[0-9a-f]{64}$/i.test(value)) {
    throw new Error(`Invalid network configuration: ${key} must be a 32-byte hex program ID.`);
  }

  return value.toLowerCase();
};

const startBlock = (): number => {
  const value = requiredNetworkEnv('GEAR_FROM_BLOCK');
  const block = Number(value);

  if (!/^\d+$/.test(value) || !Number.isSafeInteger(block)) {
    throw new Error('Invalid network configuration: GEAR_FROM_BLOCK must be a non-negative safe integer.');
  }

  return block;
};

let apiPath = getEnv('GEAR_API_PATH', './api/gear');

apiPath = apiPath.endsWith('/') ? apiPath.slice(0, -1) : apiPath;

export const config = {
  rpcUrl: networkUrl('GEAR_RPC_URL', ['ws:', 'wss:', 'http:', 'https:']),
  archiveUrl: networkUrl('GEAR_ARCHIVE_URL', ['http:', 'https:']),
  fromBlock: startBlock(),
  vftManager: programId('GEAR_VFT_MANAGER'),
  historicalProxy: programId('GEAR_HISTORICAL_PROXY'),
  bridgingPayment: programId('GEAR_BRIDGING_PAYMENT'),
  checkpointClient: programId('GEAR_CHECKPOINT_CLIENT'),
  stateSchema: getEnv('GEAR_STATE_SCHEMA', 'gear_processor'),
  ethRpcUrl: getEnv('ETH_HTTP_RPC_URL'),
  rateLimit: Number(getEnv('GEAR_RATE_LIMIT', '100')),
  apiPath,
  apiKey: getEnv('GEAR_SQUID_API_KEY'),
};
