import { createHash } from 'node:crypto';

export const createPairHash = (sourceAddress: string, destinationAddress: string) =>
  createHash('sha256')
    .update(sourceAddress + destinationAddress)
    .digest()
    .subarray(0, 16)
    .toString('hex');
