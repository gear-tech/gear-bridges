import EthSVG from '@/assets/eth.svg?react';
import VaraSVG from '@/assets/vara.svg?react';

import { Network } from '../types';

const NETWORK_SVG = {
  [Network.Gear]: VaraSVG,
  [Network.Ethereum]: EthSVG,
} as const;

export { NETWORK_SVG };
