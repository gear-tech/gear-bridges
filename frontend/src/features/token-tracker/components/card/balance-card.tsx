import { PropsWithChildren } from 'react';

import TokenPlaceholderSVG from '@/assets/token-placeholder.svg?react';
import { FormattedBalance, Skeleton } from '@/components';
import { SVGComponent } from '@/types';
import { cx } from '@/utils';

import styles from './balance-card.module.scss';

type Props = PropsWithChildren & {
  SVG: SVGComponent;
  value: bigint;
  decimals: number;
  symbol: string;
  locked?: boolean;
};

function BalanceCard({ locked, value, decimals, SVG, symbol, children }: Props) {
  return (
    <div className={cx(styles.card, locked && styles.locked)}>
      <span className={styles.balance}>
        <SVG />
        <FormattedBalance value={value} decimals={decimals} symbol={symbol} />
      </span>

      {children}
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
