import { CSSProperties } from 'react';

import EthSVG from '@/assets/eth.svg?react';
import TokenPlaceholderSVG from '@/assets/token-placeholder.svg?react';
import UsdcSVG from '@/assets/usdc.svg?react';
import UsdtSVG from '@/assets/usdt.svg?react';
import VaraUsdcSVG from '@/assets/vara-usdc.svg?react';
import VaraUsdtSVG from '@/assets/vara-usdt.svg?react';
import VaraSVG from '@/assets/vara.svg?react';
import WrappedEthSVG from '@/assets/wrapped-eth.svg?react';
import WrappedVaraSVG from '@/assets/wrapped-vara.svg?react';
import { NETWORK } from '@/features/swap/consts';

import { Skeleton } from '../layout';

import styles from './token-svg.module.scss';

const NETWORK_SVG = {
  [NETWORK.VARA]: VaraSVG,
  [NETWORK.ETH]: EthSVG,
} as const;

type Props = {
  symbol: string | undefined;
  network: 'vara' | 'eth';
  sizes: [number, number?];
};

function TokenSVG({ symbol, network, sizes }: Props) {
  const [size, networkSize = 0] = sizes;
  const style = { '--size': `${size}px`, '--network-size': `${networkSize}px` } as CSSProperties;

  const getSVG = () => {
    if (!symbol) return Skeleton;

    const lowerCaseSymbol = symbol.toLowerCase();

    if (network === NETWORK.VARA) {
      if (lowerCaseSymbol.includes('vara')) return VaraSVG;
      if (lowerCaseSymbol.includes('eth')) return WrappedEthSVG;
      if (lowerCaseSymbol.includes('usdc')) return VaraUsdcSVG;
      if (lowerCaseSymbol.includes('usdt')) return VaraUsdtSVG;
    }

    if (network === NETWORK.ETH) {
      if (lowerCaseSymbol.includes('vara')) return WrappedVaraSVG;
      if (lowerCaseSymbol.includes('eth')) return EthSVG;
      if (lowerCaseSymbol.includes('usdc')) return UsdcSVG;
      if (lowerCaseSymbol.includes('usdt')) return UsdtSVG;
    }

    return TokenPlaceholderSVG;
  };

  const SVG = getSVG();
  const NetworkSVG = NETWORK_SVG[network];

  return (
    <div className={styles.container} style={style}>
      <SVG className={styles.tokenSvg} />

      {Boolean(networkSize) && <NetworkSVG className={styles.networkSvg} />}
    </div>
  );
}

function TokenSVGSkeleton({ sizes }: Pick<Props, 'sizes'>) {
  const [size, networkSize = 0] = sizes;
  const style = { '--size': `${size}px`, '--network-size': `${networkSize}px` } as CSSProperties;

  return (
    <div className={styles.container} style={style}>
      <Skeleton className={styles.tokenSvg} />

      {Boolean(networkSize) && <Skeleton className={styles.networkSvg} />}
    </div>
  );
}

TokenSVG.Skeleton = TokenSVGSkeleton;

export { TokenSVG };
