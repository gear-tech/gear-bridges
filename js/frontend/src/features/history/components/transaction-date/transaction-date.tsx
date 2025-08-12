import ClockSVG from '@/assets/clock.svg?react';
import { Skeleton } from '@/components';
import { cx } from '@/utils';

import { Transfer } from '../../types';

import styles from './transaction-date.module.scss';

type Props = Pick<Transfer, 'timestamp'> & {
  className?: string;
};

function TransactionDate({ timestamp, className }: Props) {
  const date = new Date(timestamp).toLocaleString();

  return (
    <p className={cx(styles.date, className)}>
      <ClockSVG /> {date}
    </p>
  );
}

function TransactionDateSkeleton() {
  return (
    <p className={styles.date}>
      <Skeleton>
        <ClockSVG />
      </Skeleton>

      <Skeleton>
        <span>{new Date().toLocaleString()}</span>
      </Skeleton>
    </p>
  );
}

TransactionDate.Skeleton = TransactionDateSkeleton;

export { TransactionDate };
