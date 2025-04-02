import { CSSProperties } from 'react';

import { Card } from '@/components';
import { cx } from '@/utils';

import { UseHandleSubmit } from '../../types';
import { getErrorMessage } from '../../utils';

import styles from './submit-progress-bar.module.scss';

type Props = {
  submit: Omit<ReturnType<UseHandleSubmit>[0], 'mutateAsync'>;
  approve: ReturnType<UseHandleSubmit>[1];
  mint: ReturnType<UseHandleSubmit>[2];
};

const PERCENTAGE = {
  default: 0,
  mint: 25,
  approve: 50,
  transfer: 75,
  success: 100,
} as const;

const TEXT = {
  default: '',
  mint: 'Locking',
  approve: 'Approving',
  transfer: 'Transferring',
  success: 'Your transfer request was successful',
} as const;

const ERROR_TEXT = {
  default: '',
  mint: 'Lock',
  approve: 'Approve',
  transfer: 'Transfer',
  success: '',
} as const;

function SubmitProgressBar({ mint, approve, submit }: Props) {
  const { isSuccess, isPending, error } = submit;
  const errorMessage = error ? getErrorMessage(error) : '';

  const getStatus = () => {
    if (mint?.isPending || mint?.error) return 'mint';
    if (approve.isPending || approve.error) return 'approve';
    if (submit.isPending || submit.error) return 'transfer';
    if (isSuccess) return 'success';
    return 'default';
  };

  const status = getStatus();

  return (
    <Card className={cx(styles.container, isPending && styles.loading, errorMessage && styles.error)}>
      <p className={styles.text}>
        {errorMessage ? `${ERROR_TEXT[status]} transaction failed: ${errorMessage}` : TEXT[status]}
      </p>

      <div className={styles.bar} style={{ '--width': `${PERCENTAGE[status]}%` } as CSSProperties} />
    </Card>
  );
}

export { SubmitProgressBar };
