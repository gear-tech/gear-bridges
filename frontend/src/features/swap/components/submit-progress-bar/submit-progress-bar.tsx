import { CSSProperties } from 'react';

import { cx } from '@/utils';

import styles from './submit-progress-bar.module.scss';

type Props = {
  status: 'mint' | 'approve' | 'transfer';
  error: string;
  isSuccess: boolean;
};

const PERCENTAGE = {
  mint: 25,
  approve: 50,
  transfer: 75,
} as const;

const TEXT = {
  mint: 'Locking',
  approve: 'Approving',
  transfer: 'Transferring',
} as const;

const ERROR_TEXT = {
  mint: 'Lock',
  approve: 'Approve',
  transfer: 'Transfer',
} as const;

function SubmitProgressBar({ status, error, isSuccess }: Props) {
  const getClassName = () => {
    if (isSuccess) return styles.success;
    if (error) return styles.error;
  };

  return (
    <div className={cx(styles.container, getClassName())}>
      <p className={styles.text}>{error ? `${ERROR_TEXT[status]} failed: ${error}` : TEXT[status]}</p>

      <div className={styles.bar} style={{ '--width': `${PERCENTAGE[status]}%` } as CSSProperties} />
    </div>
  );
}

export { SubmitProgressBar };
