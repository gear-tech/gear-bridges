import { CSSProperties } from 'react';

import { Card } from '@/components';
import { cx, getErrorMessage } from '@/utils';

import { SUBMIT_STATUS } from '../../consts';
import { UseHandleSubmit } from '../../types';

import styles from './submit-progress-bar.module.scss';

type Props = Pick<ReturnType<UseHandleSubmit>, 'status' | 'isPending' | 'error'> & {
  isVaraNetwork: boolean;
};

const VARA_PERCENTAGE = {
  [SUBMIT_STATUS.MINT]: 0,
  [SUBMIT_STATUS.APPROVE]: 0,
  [SUBMIT_STATUS.PERMIT]: 0,
  [SUBMIT_STATUS.BRIDGE]: 50,
  [SUBMIT_STATUS.FEE]: 75,
  [SUBMIT_STATUS.SUCCESS]: 100,
} as const;

const ETH_PERCENTAGE = {
  [SUBMIT_STATUS.FEE]: 0,
  [SUBMIT_STATUS.MINT]: 25,
  [SUBMIT_STATUS.APPROVE]: 50,
  [SUBMIT_STATUS.PERMIT]: 50,
  [SUBMIT_STATUS.BRIDGE]: 75,
  [SUBMIT_STATUS.SUCCESS]: 100,
} as const;

const TEXT = {
  [SUBMIT_STATUS.MINT]: 'Locking tokens',
  [SUBMIT_STATUS.APPROVE]: 'Approving tokens',
  [SUBMIT_STATUS.PERMIT]: 'Requesting signature to permit token spending',
  [SUBMIT_STATUS.BRIDGE]: 'Requesting transfer',
  [SUBMIT_STATUS.FEE]: 'Paying fee',
  [SUBMIT_STATUS.SUCCESS]: 'Your transfer request have been successful',
} as const;

const ERROR_TEXT = {
  [SUBMIT_STATUS.MINT]: 'Tokens lock',
  [SUBMIT_STATUS.APPROVE]: 'Tokens approval',
  [SUBMIT_STATUS.PERMIT]: 'Permit signature',
  [SUBMIT_STATUS.BRIDGE]: 'Transfer request',
  [SUBMIT_STATUS.FEE]: 'Fee payment',
  [SUBMIT_STATUS.SUCCESS]: '',
} as const;

function SubmitProgressBar({ isVaraNetwork, status, isPending, error }: Props) {
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
