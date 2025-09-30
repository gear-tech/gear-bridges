import * as crypto from 'node:crypto';

export const ethNonce = (data: string) => crypto.createHash('sha256').update(data).digest('hex');

export const gearNonce = (data: bigint) => {
  return data.toString();
};
