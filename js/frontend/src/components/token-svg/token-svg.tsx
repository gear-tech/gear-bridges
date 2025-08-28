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
import { cx } from '@/utils';

import { Skeleton } from '../layout';

import styles from './token-svg.module.scss';

const NETWORK_SVG = {
  [NETWORK.VARA]: VaraSVG,
  [NETWORK.ETH]: EthSVG,
} as const;

type Props = {
  symbol: string | undefined;
  network: 'vara' | 'eth';
  className?: string;
  displayNetwork?: boolean;
};

function TokenSVG({ symbol, network, className, displayNetwork = true }: Props) {
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
    <div className={cx(styles.container, className)}>
      <SVG className={styles.tokenSvg} />

      {displayNetwork && <NetworkSVG className={styles.networkSvg} />}
    </div>
  );
}

function TokenSVGSkeleton({ className, displayNetwork = true }: Pick<Props, 'displayNetwork' | 'className'>) {
  return (
    <div className={cx(styles.container, className)}>
      <Skeleton className={styles.tokenSvg} />

      {displayNetwork && <Skeleton className={styles.networkSvg} />}
    </div>
  );
}

TokenSVG.Skeleton = TokenSVGSkeleton;

export { TokenSVG };
