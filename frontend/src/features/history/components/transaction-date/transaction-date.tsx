import { cx } from '@/utils';

import ClockSVG from '../../assets/clock.svg?react';
import { Transfer } from '../../types';

import styles from './transaction-date.module.scss';

type Props = Pick<Transfer, 'timestamp'> & {
  isCompact?: boolean;
};

function TransactionDate({ timestamp, isCompact }: Props) {
  const date = new Date(timestamp).toLocaleString();

  return (
    <p className={cx(styles.date, isCompact && styles.compact)}>
      <ClockSVG /> {date}
    </p>
  );
}

function TransactionDateSkeleton() {
  return <p className={styles.date}>Date</p>;
}

TransactionDate.Skeleton = TransactionDateSkeleton;

export { TransactionDate };
