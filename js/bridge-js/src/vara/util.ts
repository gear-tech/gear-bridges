import { TypeRegistry } from '@polkadot/types';

const registry = new TypeRegistry();

export const getPrefix = (service: string, method: string): `0x${string}` => {
  return registry.createType('(String, String)', [service, method]).toHex();
};
