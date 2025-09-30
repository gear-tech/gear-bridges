import { u8aConcat, bnToU8a } from '@polkadot/util';
import { keccak256 } from 'viem';

import { VaraMessage } from '../vara/types.js';

export const messageHash = (msg: VaraMessage) => {
  const nonceBe = bnToU8a(msg.nonce, { bitLength: 256, isLe: false });
  const bytes = u8aConcat(nonceBe, msg.source, msg.destination, msg.payload);
  return keccak256(bytes);
};
