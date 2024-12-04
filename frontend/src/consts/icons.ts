import { HexString } from '@gear-js/api';

import EthSVG from '@/assets/eth.svg?react';
import UsdcSVG from '@/assets/usdc.svg?react';
import VaraUsdcSVG from '@/assets/vara-usdc.svg?react';
import VaraSVG from '@/assets/vara.svg?react';
import WrappedEthSVG from '@/assets/wrapped-eth.svg?react';
import WrappedVaraSVG from '@/assets/wrapped-vara.svg?react';
import { SVGComponent } from '@/types';

const TOKEN_SVG: Record<HexString, SVGComponent> = {
  '0x00': VaraSVG,
  '0x01': EthSVG,
  '0x02': WrappedVaraSVG,
  '0x03': WrappedEthSVG,
  '0x05': VaraUsdcSVG,
  '0x04': UsdcSVG,
};

export { TOKEN_SVG };
