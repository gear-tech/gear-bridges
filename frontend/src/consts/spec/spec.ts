import EthSVG from '@/assets/eth.svg?react';
import UsdcSVG from '@/assets/usdc.svg?react';
import VaraUsdcSVG from '@/assets/vara-usdc.svg?react';
import VaraSVG from '@/assets/vara.svg?react';
import WrappedEthSVG from '@/assets/wrapped-eth.svg?react';
import WrappedVaraSVG from '@/assets/wrapped-vara.svg?react';

import { NATIVE_SYMBOL, NETWORK_NAME } from './network';

const SPEC = {
  VaraWrappedVara: {
    [NETWORK_NAME.VARA]: {
      address: '0x46a599fc64e2d8c8765491a8ca3cc2ab11e7c3a32e1dbd9cb14681a3db09d1b6',
      symbol: NATIVE_SYMBOL.VARA,
      tokenType: 'native',
      decimals: 12,
      SVG: VaraSVG,
    },
    [NETWORK_NAME.ETH]: {
      address: '0xEb51145B04d7fF94aEF976f74eAE879baC21A9D5',
      symbol: 'WVARA',
      tokenType: 'fungible',
      decimals: 12,
      SVG: WrappedVaraSVG,
    },
  },

  EthWrappedEth: {
    [NETWORK_NAME.VARA]: {
      address: '0xbc22e51aedfcb8d4a580483b51cb627baa9b54f2d40e8dd7629f580e1571cee7',
      symbol: 'WETH',
      tokenType: 'fungible',
      decimals: 18,
      SVG: WrappedEthSVG,
    },
    [NETWORK_NAME.ETH]: {
      address: '0x9d2eB695a2Ea57e40b81FB99fd7f80C8Eb98ecFa',
      symbol: NATIVE_SYMBOL.ETH,
      tokenType: 'native',
      decimals: 18,
      SVG: EthSVG,
    },
  },

  USDCWrappedUSDC: {
    [NETWORK_NAME.VARA]: {
      address: '0xf4c486a710ae0fb67acee5c2665f989b39ce99329fa56e8f9fac0e1f71ce065f',
      symbol: 'VUSDC',
      tokenType: 'fungible',
      decimals: 6,
      SVG: VaraUsdcSVG,
    },
    [NETWORK_NAME.ETH]: {
      address: '0xDfC73A8AFE32508a4fd1e055427C9C5093108322',
      symbol: 'USDC',
      tokenType: 'fungible',
      decimals: 6,
      SVG: UsdcSVG,
    },
  },
} as const;

export { SPEC };
