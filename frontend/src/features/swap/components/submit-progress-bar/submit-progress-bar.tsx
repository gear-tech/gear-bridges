import { CSSProperties } from 'react';

import { Card } from '@/components';
import { cx, getErrorMessage } from '@/utils';

import { UseHandleSubmit } from '../../types';

import styles from './submit-progress-bar.module.scss';

type Props = ReturnType<UseHandleSubmit>;

const VARA_PERCENTAGE = {
  default: 0,
  mint: 0,
  approve: 0,
  permitUSDC: 0,
  transfer: 50,
  fee: 75,
  success: 100,
} as const;

const ETH_PERCENTAGE = {
  default: 0,
  fee: 0,
  mint: 25,
  approve: 50,
  permitUSDC: 50,
  transfer: 75,
  success: 100,
} as const;

const TEXT = {
  default: '',
  mint: 'Locking tokens',
  approve: 'Approving tokens',
  permitUSDC: 'Requesting signature to permit token spending',
  transfer: 'Requesting transfer',
  fee: 'Paying fee',
  success: 'Your transfer request and fee payment have been successful',
} as const;

const ERROR_TEXT = {
  default: '',
  mint: 'Tokens lock',
  approve: 'Tokens approval',
  permitUSDC: 'Permit signature',
  transfer: 'Transfer request',
  fee: 'Fee payment',
  success: '',
} as const;

function SubmitProgressBar({ approve, submit, payFee, mint, permitUSDC }: Props) {
  const { isSuccess, isPending, error } = submit;
  const errorMessage = error ? getErrorMessage(error) : '';

  const getStatus = () => {
    if (mint?.isPending || mint?.error) return 'mint';
    if (payFee?.isPending || payFee?.error) return 'fee';
    if (permitUSDC?.isPending || permitUSDC?.error) return 'permitUSDC';
    if (approve?.isPending || approve?.error) return 'approve';
    if (submit.isPending || submit.error) return 'transfer';
    if (isSuccess) return 'success';
    return 'default';
  };

  const status = getStatus();

  return (
    <Card className={cx(styles.container, isPending && styles.loading, errorMessage && styles.error)}>
      <p className={styles.text}>{errorMessage ? `${ERROR_TEXT[status]} failed: ${errorMessage}` : TEXT[status]}</p>

      <div
        className={styles.bar}
        style={{ '--width': `${mint ? ETH_PERCENTAGE[status] : VARA_PERCENTAGE[status]}%` } as CSSProperties}
      />
    </Card>
  );
}

export { SubmitProgressBar };
