import { CSSProperties } from 'react';

import { Card } from '@/components';
import { cx, getErrorMessage } from '@/utils';

import { UseHandleSubmit } from '../../types';

import styles from './submit-progress-bar.module.scss';

type Props = Pick<ReturnType<UseHandleSubmit>, 'status' | 'isPending' | 'error'> & {
  isVaraNetwork: boolean;
};

const VARA_PERCENTAGE = {
  mint: 0,
  approve: 0,
  permit: 0,
  bridge: 50,
  fee: 75,
  success: 100,
} as const;

const ETH_PERCENTAGE = {
  fee: 0,
  mint: 25,
  approve: 50,
  permit: 50,
  bridge: 75,
  success: 100,
} as const;

const TEXT = {
  mint: 'Locking tokens',
  approve: 'Approving tokens',
  permit: 'Requesting signature to permit token spending',
  bridge: 'Requesting transfer',
  fee: 'Paying fee',
  success: 'Your transfer request and fee payment have been successful',
} as const;

const ERROR_TEXT = {
  mint: 'Tokens lock',
  approve: 'Tokens approval',
  permit: 'Permit signature',
  bridge: 'Transfer request',
  fee: 'Fee payment',
  success: '',
} as const;

function SubmitProgressBar({ isVaraNetwork, status, isPending, error }: Props) {
  console.log('status: ', status);
  const errorMessage = error ? getErrorMessage(error) : '';

  return (
    <Card className={cx(styles.container, isPending && styles.loading, errorMessage && styles.error)}>
      <p className={styles.text}>{errorMessage ? `${ERROR_TEXT[status]} failed: ${errorMessage}` : TEXT[status]}</p>

      <div
        className={styles.bar}
        style={{ '--width': `${isVaraNetwork ? VARA_PERCENTAGE[status] : ETH_PERCENTAGE[status]}%` } as CSSProperties}
      />
    </Card>
  );
}

export { SubmitProgressBar };
