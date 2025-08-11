import ClockSVG from '@/assets/clock.svg?react';
import { Skeleton } from '@/components';
import { cx } from '@/utils';

import CheckSVG from '../../assets/check.svg?react';
import ErrorSVG from '../../assets/error.svg?react';
import { Transfer, Status as StatusType } from '../../types';

import styles from './transaction-status.module.scss';

const STATUS_SVG = {
  [StatusType.Completed]: CheckSVG,
  [StatusType.AwaitingPayment]: ClockSVG,
  [StatusType.Failed]: ErrorSVG,
  [StatusType.Bridging]: ClockSVG,
} as const;

const STATUS_CLASSNAME = {
  [StatusType.Completed]: styles.completed,
  [StatusType.AwaitingPayment]: styles.awaitingPayment,
  [StatusType.Failed]: styles.failed,
  [StatusType.Bridging]: styles.bridging,
};

const STATUS_TEXT = {
  [StatusType.Completed]: 'Completed',
  [StatusType.AwaitingPayment]: 'Awaiting Payment',
  [StatusType.Failed]: 'Failed',
  [StatusType.Bridging]: 'Bridging',
} as const;

function TransactionStatus({ status }: Pick<Transfer, 'status'>) {
  const StatusSVG = STATUS_SVG[status];

  return (
    <div className={cx(styles.status, STATUS_CLASSNAME[status])}>
      <StatusSVG />
      {STATUS_TEXT[status] || status}
    </div>
  );
}

function TransactionStatusSkeleton() {
  return (
    <Skeleton className={styles.status}>
      <span>Complete</span>
    </Skeleton>
  );
}

TransactionStatus.Skeleton = TransactionStatusSkeleton;

export { TransactionStatus };
