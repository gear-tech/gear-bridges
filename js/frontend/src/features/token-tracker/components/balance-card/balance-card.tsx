import { PropsWithChildren } from 'react';

import TokenPlaceholderSVG from '@/assets/token-placeholder.svg?react';
import { FormattedBalance, Skeleton, TokenSVG } from '@/components';

import styles from './balance-card.module.scss';

type Props = PropsWithChildren & {
  value: bigint;
  decimals: number;
  symbol: string;
  network: 'vara' | 'eth';
};

function BalanceCard({ value, decimals, symbol, network, children }: Props) {
  return (
    <div className={styles.card}>
      <span className={styles.balance}>
        <TokenSVG symbol={symbol} network={network} displayNetwork={false} />
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
