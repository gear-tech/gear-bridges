import * as dotenv from 'dotenv';
import { resolve, join } from 'node:path';

dotenv.config();

export const getEnv = (key: string, _default?: string): string => {
  const env = process.env[key] || _default;

  if (!env) {
    throw new Error(`Missing env: ${key}`);
  }

  return env;
};

export type EvidenceOwner = 'eth' | 'gear' | 'repair';

export function getEthereumEvidenceConfig(owner: EvidenceOwner = 'eth'): {
  rpcUrl: string; chainId: bigint; genesisHash: string; binary: string; beaconRpc: string;
  bootstrap: string; bootstrapSha256: string; store: string; proofSources: string[]; evidenceProofSources: string[];
} {
  const endpoint = (key: string): string => {
    const value = getEnv(key);
    let url: URL;
    try { url = new URL(value); } catch { throw new Error(`Invalid ${key}: expected an HTTP(S) endpoint`); }
    if (!['http:', 'https:'].includes(url.protocol) || !url.hostname || url.hash) {
      throw new Error(`Invalid ${key}: expected an HTTP(S) endpoint without a fragment`);
    }
    const loopback = url.hostname === '127.0.0.1' || url.hostname === '[::1]';
    if (url.protocol === 'http:' && !loopback) throw new Error(`Invalid ${key}: HTTPS required except literal loopback`);
    return url.toString();
  };
  const chainId = getEnv('ETH_CHAIN_ID');
  if (!/^(0|[1-9][0-9]*)$/.test(chainId) || BigInt(chainId) > 0xffffffffffffffffn) {
    throw new Error('Invalid ETH_CHAIN_ID: expected a decimal uint64');
  }
  const genesisHash = getEnv('ETH_GENESIS_HASH');
  const bootstrapSha256 = getEnv('ETH_FINALITY_BOOTSTRAP_SHA256');
  if (!/^0x[0-9a-f]{64}$/.test(genesisHash) || !/^0x[0-9a-f]{64}$/.test(bootstrapSha256)) {
    throw new Error('Invalid Ethereum genesis or bootstrap approval digest');
  }
  const root = resolve(getEnv('ETH_FINALITY_ARCHIVE_ROOT'));
  const store = join(root, owner);
  const evidenceProofSources = (['eth', 'gear', 'repair'] as const).filter((peer) => peer !== owner)
    .map((peer) => join(root, peer));
  const proofSources = evidenceProofSources.map((peer) => join(peer, 'beacon'));
  return { rpcUrl: endpoint('ETH_RPC_URL'), chainId: BigInt(chainId), genesisHash,
    binary: getEnv('ETH_EVIDENCE_BINARY', 'relayer'), beaconRpc: endpoint('ETH_FINALITY_BEACON_RPC'),
    bootstrap: getEnv('ETH_FINALITY_BOOTSTRAP'), bootstrapSha256, store, proofSources, evidenceProofSources };
}
