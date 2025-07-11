import { Codec, TypeKind } from '@subsquid/scale-codec';
import { hexToBigInt } from '@polkadot/util';
import * as crypto from 'node:crypto';

export const ethNonce = (data: string) => crypto.createHash('sha256').update(data).digest('hex');

const codec = new Codec([{ kind: TypeKind.Primitive, primitive: 'U256' }]);

export const gearNonce = (data: string, isLe = true) => {
  let nonce = codec.encodeToHex(0, hexToBigInt(data, { isLe })).slice(2);
  while (nonce.startsWith('00')) {
    nonce = nonce.slice(2);
  }
  return nonce;
};

export const gearNonceFromNumber = (data: string) => {
  const nonce = '0x' + BigInt(data).toString(16).padStart(64, '0');
  return gearNonce(nonce);
};
