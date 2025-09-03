import { Link } from 'react-router-dom';

import WarningSVG from '@/assets/warning.svg?react';
import { ROUTE } from '@/consts';

import { usePendingTxsCount } from '../../hooks';
import { Status } from '../../types';

import styles from './pending-transactions-warning.module.scss';

function PendingTransactionsWarning() {
  const { data: txsCount } = usePendingTxsCount();

  if (!txsCount) return;

  return (
    <div className={styles.container}>
      <WarningSVG className={styles.icon} />

      <p>
        You have transactions awaiting fee payment.{' '}
        <Link to={`${ROUTE.TRANSACTIONS}?owner=true&status=${Status.AwaitingPayment}`}>Navigate</Link>
      </p>
    </div>
  );
}

export { PendingTransactionsWarning };
