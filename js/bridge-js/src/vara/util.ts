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
 * @returns Object containing the decoded nonce, hash, block number and queue id
 */
export const decodeEthBridgeMessageResponse = (
  data: Uint8Array,
): { blockNumber: bigint; hash: HexString; nonce: bigint; queueId: bigint } => {
  const _data = data.length == 76 ? data : data.slice(data.length - 76);

  const [blockNumber, hash, nonce, queueId] = registry.createType('(u32, H256, U256, u64)', _data);

  return {
    blockNumber: blockNumber.toBigInt(),
    hash: hash.toHex(),
    nonce: nonce.toBigInt(),
    queueId: queueId.toBigInt(),
  };
};
