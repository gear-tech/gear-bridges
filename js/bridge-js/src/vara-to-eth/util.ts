import { u8aConcat, bnToU8a } from '@polkadot/util';
import { keccak256 } from 'viem';

import { VaraMessage } from '../vara/types.js';

export const messageHash = (msg: VaraMessage) => {
  const nonceLe = bnToU8a(msg.nonce, { bitLength: 256, isLe: true });
  const bytes = u8aConcat(nonceLe, msg.source, msg.destination, msg.payload);
  return keccak256(bytes);
};
