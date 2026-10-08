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
  if (data.length !== 77 || data[0] !== 0) {
    throw new Error('Invalid EthBridge response: expected the exact tagged EthMessageQueued response');
  }

  const [blockNumber, hash, nonce, queueId] = registry.createType('(u32, H256, U256, u64)', data.subarray(1));

  return {
    blockNumber: blockNumber.toBigInt(),
    hash: hash.toHex(),
    nonce: nonce.toBigInt(),
    queueId: queueId.toBigInt(),
  };
};
