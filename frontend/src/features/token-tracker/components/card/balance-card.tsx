import TokenPlaceholderSVG from '@/assets/token-placeholder.svg?react';
import { Skeleton } from '@/components';
import { SVGComponent } from '@/types';
import { cx } from '@/utils';

import styles from './balance-card.module.scss';

type Props = {
  SVG: SVGComponent;
  value: string;
  symbol: string;
  locked?: boolean;
};

function BalanceCard({ locked, value, SVG, symbol }: Props) {
  return (
    <div className={cx(styles.card, locked && styles.locked)}>
      <span className={styles.balance}>
        <SVG />
        {value} {symbol}
      </span>
    </div>
  );
}

function BalanceCardSkeleton() {
  return (
    <div className={styles.card}>
      <span className={styles.balance}>
        <Skeleton borderRadius="50%">
          <TokenPlaceholderSVG />
        </Skeleton>

        <Skeleton width="5rem" />
        <Skeleton width="2.5rem" />
      </span>
    </div>
  );
}

BalanceCard.Skeleton = BalanceCardSkeleton;

export { BalanceCard };
