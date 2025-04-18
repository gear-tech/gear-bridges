import { HexString } from '@gear-js/api';

import EthSVG from '@/assets/eth.svg?react';
import UsdcSVG from '@/assets/usdc.svg?react';
import UsdtSVG from '@/assets/usdt.svg?react';
import VaraUsdcSVG from '@/assets/vara-usdc.svg?react';
import VaraUsdtSVG from '@/assets/vara-usdt.svg?react';
import VaraSVG from '@/assets/vara.svg?react';
import WrappedEthSVG from '@/assets/wrapped-eth.svg?react';
import WrappedVaraSVG from '@/assets/wrapped-vara.svg?react';
import { SVGComponent } from '@/types';

import {
  ETH_WRAPPED_ETH_CONTRACT_ADDRESS,
  ETH_WRAPPED_VARA_CONTRACT_ADDRESS,
  USDC_CONTRACT_ADDRESS,
  USDT_CONTRACT_ADDRESS,
  WRAPPED_ETH_CONTRACT_ADDRESS,
  WRAPPED_USDC_CONTRACT_ADDRESS,
  WRAPPED_USDT_CONTRACT_ADDRESS,
  WRAPPED_VARA_CONTRACT_ADDRESS,
} from './env';

const TOKEN_SVG: Record<HexString, SVGComponent> = {
  [WRAPPED_VARA_CONTRACT_ADDRESS]: VaraSVG,
  [WRAPPED_ETH_CONTRACT_ADDRESS]: WrappedEthSVG,
  [WRAPPED_USDC_CONTRACT_ADDRESS]: VaraUsdcSVG,
  [WRAPPED_USDT_CONTRACT_ADDRESS]: VaraUsdtSVG,

  [ETH_WRAPPED_ETH_CONTRACT_ADDRESS]: EthSVG,
  [ETH_WRAPPED_VARA_CONTRACT_ADDRESS]: WrappedVaraSVG,
  [USDC_CONTRACT_ADDRESS]: UsdcSVG,
  [USDT_CONTRACT_ADDRESS]: UsdtSVG,
};

export { TOKEN_SVG };
