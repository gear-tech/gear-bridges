import { Codec, TypeKind } from '@subsquid/scale-codec';
import * as crypto from 'node:crypto';

export const ethNonce = (data: string) => crypto.createHash('sha256').update(data).digest('hex');

const codec = new Codec([{ kind: TypeKind.Primitive, primitive: 'U256' }]);

export const gearNonce = (data: bigint, isLe = true) => {
  if (isLe) {
    return data.toString(16);
  }
  let nonce = codec.encodeToHex(0, data).slice(2);
  while (nonce.startsWith('00')) {
    nonce = nonce.slice(2);
  }
  return nonce;
};
