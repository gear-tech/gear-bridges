import EthSVG from '@/assets/eth.svg?react';
import TokenPlaceholderSVG from '@/assets/token-placeholder.svg?react';
import VaraSVG from '@/assets/vara.svg?react';
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

    // Metadata alone never grants an approved asset's artwork.
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
