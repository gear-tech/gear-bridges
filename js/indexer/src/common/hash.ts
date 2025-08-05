import * as crypto from 'node:crypto';

export const hash = (...data: string[]) => {
  const hash = crypto.createHash('sha256').update(data.join('')).digest();
  return hash.subarray(0, 16).toString('hex');
};
