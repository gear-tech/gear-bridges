import { HexString } from '@gear-js/api';
import { TypeRegistry } from '@polkadot/types';

const registry = new TypeRegistry();

export const getPrefix = (service: string, method: string): `0x${string}` => {
  return registry.createType('(String, String)', [service, method]).toHex();
};

/**
 * Decodes a response from EthBridge builtin
 *
 * @param data - The raw data bytes containing the encoded message response
 * @returns Object containing the decoded nonce, hash, and nonce in little endian format
 */
export const decodeEthBridgeMessageResponse = (
  data: Uint8Array,
): { nonce: bigint; hash: HexString; nonceLe: HexString } => {
  const _data = data.length == 64 ? data : data.slice(data.length - 64);

  const [nonce, hash] = registry.createType('(U256, H256)', _data);

  return {
    nonce: nonce.toBigInt(),
    hash: hash.toHex(),
    nonceLe: nonce.toHex(true),
  };
};
