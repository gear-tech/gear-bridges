import { createHash } from 'node:crypto';

const getPairHash = (sourceAddress: string, destinationAddress: string) =>
  createHash('sha256')
    .update(sourceAddress + destinationAddress)
    .digest()
    .subarray(0, 16)
    .toString('hex');

export { getPairHash };
