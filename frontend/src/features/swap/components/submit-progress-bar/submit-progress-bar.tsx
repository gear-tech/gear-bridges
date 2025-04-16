import { CSSProperties } from 'react';

import { Card } from '@/components';
import { cx } from '@/utils';

import { UseHandleSubmit } from '../../types';
import { getErrorMessage } from '../../utils';

import styles from './submit-progress-bar.module.scss';

type Props = {
  submit: Omit<ReturnType<UseHandleSubmit>[0], 'mutateAsync'>;
  approve: ReturnType<UseHandleSubmit>[1];
  payFee: ReturnType<UseHandleSubmit>[2];
};

const PERCENTAGE = {
  default: 0,
  approve: 25,
  transfer: 50,
  fee: 75,
  success: 100,
} as const;

const TEXT = {
  default: '',
  approve: 'Approving',
  transfer: 'Requesting transfer',
  fee: 'Paying fee',
  success: 'Your transfer request and fee payment have been successful',
} as const;

const ERROR_TEXT = {
  default: '',
  approve: 'Approve',
  transfer: 'Transfer request',
  fee: 'Fee payment',
  success: '',
} as const;

function SubmitProgressBar({ approve, submit, payFee }: Props) {
  const { isSuccess, isPending, error } = submit;
  const errorMessage = error ? getErrorMessage(error) : '';

  const getStatus = () => {
    if (payFee?.isPending || payFee?.error) return 'fee';
    if (approve.isPending || approve.error) return 'approve';
    if (submit.isPending || submit.error) return 'transfer';
    if (isSuccess) return 'success';
    return 'default';
  };

  const status = getStatus();

  return (
    <Card className={cx(styles.container, isPending && styles.loading, errorMessage && styles.error)}>
      <p className={styles.text}>{errorMessage ? `${ERROR_TEXT[status]} failed: ${errorMessage}` : TEXT[status]}</p>

      <div className={styles.bar} style={{ '--width': `${PERCENTAGE[status]}%` } as CSSProperties} />
    </Card>
  );
}

export { SubmitProgressBar };
