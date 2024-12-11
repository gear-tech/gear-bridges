import ClockSVG from '@/assets/clock.svg?react';
import { Skeleton } from '@/components';
import { cx } from '@/utils';

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

function TransactionDateSkeleton({ isCompact }: Pick<Props, 'isCompact'>) {
  return (
    <p className={cx(styles.date, isCompact && styles.compact)}>
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
