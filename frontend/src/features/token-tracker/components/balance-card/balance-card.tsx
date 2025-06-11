import { PropsWithChildren } from 'react';

import TokenPlaceholderSVG from '@/assets/token-placeholder.svg?react';
import { FormattedBalance, Skeleton, TokenSVG } from '@/components';
import { cx } from '@/utils';

import styles from './balance-card.module.scss';

type Props = PropsWithChildren & {
  value: bigint;
  decimals: number;
  symbol: string;
  networkIndex: number;
  locked?: boolean;
};

function BalanceCard({ locked, value, decimals, symbol, networkIndex, children }: Props) {
  return (
    <div className={cx(styles.card, locked && styles.locked)}>
      <span className={styles.balance}>
        <TokenSVG symbol={symbol} networkIndex={networkIndex} sizes={[24]} />
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
