import ClockSVG from '@/assets/clock.svg?react';
import { Skeleton } from '@/components';
import { cx } from '@/utils';

import CheckSVG from '../../assets/check.svg?react';
import ErrorSVG from '../../assets/error.svg?react';
import { Transfer, Status as StatusType } from '../../types';

import styles from './transaction-status.module.scss';

const STATUS_SVG = {
  [StatusType.Completed]: CheckSVG,
  [StatusType.InProgress]: ClockSVG,
  [StatusType.Failed]: ErrorSVG,
  [StatusType.Pending]: ClockSVG,
} as const;

function TransactionStatus({ status }: Pick<Transfer, 'status'>) {
  const StatusSVG = STATUS_SVG[status];

  return (
    <div className={cx(styles.status, styles[status])}>
      <StatusSVG />
      {status.split(/(?=[A-Z])/).join(' ')}
    </div>
  );
}

function TransactionStatusSkeleton() {
  return <Skeleton className={styles.status} />;
}

TransactionStatus.Skeleton = TransactionStatusSkeleton;

export { TransactionStatus };
