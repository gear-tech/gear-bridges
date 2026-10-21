import * as dotenv from 'dotenv';

dotenv.config();

export const getEnv = (key: string, _default?: string): string => {
  const env = process.env[key] || _default;

  if (!env) {
    throw new Error(`Missing env: ${key}`);
  }

  return env;
};

// These endpoints must be operated independently; distinct URLs alone cannot prove that.
export function getEthereumEvidenceConfig(): {
  rpcUrl: string; witnessRpcUrl: string; chainId: bigint; genesisHash: string;
} {
  const endpoint = (key: string): string => {
    const value = getEnv(key);
    let url: URL;
    try { url = new URL(value); } catch { throw new Error(`Invalid ${key}: expected an HTTP(S) endpoint`); }
    if (!['http:', 'https:'].includes(url.protocol) || !url.hostname || url.hash) {
      throw new Error(`Invalid ${key}: expected an HTTP(S) endpoint without a fragment`);
    }
    return url.toString();
  };
  const rpcUrl = endpoint('ETH_RPC_URL');
  const witnessRpcUrl = endpoint('ETH_WITNESS_RPC');
  if (rpcUrl.replace(/\/$/, '') === witnessRpcUrl.replace(/\/$/, '')) {
    throw new Error('ETH_WITNESS_RPC must be independent of ETH_RPC_URL');
  }
  const chainId = getEnv('ETH_CHAIN_ID');
  if (!/^(0|[1-9][0-9]*)$/.test(chainId) || BigInt(chainId) > 0xffffffffffffffffn) {
    throw new Error('Invalid ETH_CHAIN_ID: expected a decimal uint64');
  }
  const genesisHash = getEnv('ETH_GENESIS_HASH');
  if (!/^0x[0-9a-f]{64}$/i.test(genesisHash)) {
    throw new Error('Invalid ETH_GENESIS_HASH: expected a 32-byte hash');
  }
  return { rpcUrl, witnessRpcUrl, chainId: BigInt(chainId), genesisHash: genesisHash.toLowerCase() };
}
