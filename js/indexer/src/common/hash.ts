import xxhash from 'xxhash-addon';

export const hash = (...args: (string | number | bigint)[]) => {
  const message = Buffer.from(args.join(''));
  return xxhash.XXHash3.hash(message).toString('hex');
};
