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
      address: '0x7aef50c8e4e641c34fa16ee03bc77f4e58c5aa47a0e1a6ad501aa39a3109d7bd',
      symbol: NATIVE_SYMBOL.VARA,
      tokenType: 'native',
      decimals: 12,
      SVG: VaraSVG,
    },
    [NETWORK_NAME.ETH]: {
      address: '0x8e12DDfcEA92B054Ea43E1a451DB111BaAaab4F4',
      symbol: 'WVARA',
      tokenType: 'fungible',
      decimals: 12,
      SVG: WrappedVaraSVG,
    },
  },

  EthWrappedEth: {
    [NETWORK_NAME.VARA]: {
      address: '0x79b9575138933425e32a2b04b54134566ff54263a9b3c2fdb2fe33f0f7a57160',
      symbol: 'WETH',
      tokenType: 'fungible',
      decimals: 18,
      SVG: WrappedEthSVG,
    },
    [NETWORK_NAME.ETH]: {
      address: '0x0db12bb862125B8798098F619eE50e812A3800C8',
      symbol: NATIVE_SYMBOL.ETH,
      tokenType: 'native',
      decimals: 18,
      SVG: EthSVG,
    },
  },

  USDCWrappedUSDC: {
    [NETWORK_NAME.VARA]: {
      address: '0x1b9701dfffe53de8a63991557a75fd4528aafec466b1e335ce51f4dcfeb4d4c4',
      symbol: 'VUSDC',
      tokenType: 'fungible',
      decimals: 6,
      SVG: VaraUsdcSVG,
    },
    [NETWORK_NAME.ETH]: {
      address: '0xc17163E4BC023B0c50DE1d32Ff7c87284fD89883',
      symbol: 'USDC',
      tokenType: 'fungible',
      decimals: 6,
      SVG: UsdcSVG,
    },
  },
} as const;

export { SPEC };
