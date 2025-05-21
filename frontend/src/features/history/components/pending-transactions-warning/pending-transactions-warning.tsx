import { useAccount } from '@gear-js/react-hooks';
import { Link } from 'react-router-dom';

import WarningSVG from '@/assets/warning.svg?react';
import { ROUTE } from '@/consts';

import { useTransactionsCount } from '../../hooks';
import { Status, TransferWhereInput } from '../../types';

import styles from './pending-transactions-warning.module.scss';

function PendingTransactionsWarning() {
  const { account } = useAccount(); // fee payment is a standalone transaction only for vara network

  const [txsCount] = useTransactionsCount(
    account
      ? ({ sender_eq: account.decodedAddress, status_eq: Status.AwaitingPayment } as TransferWhereInput)
      : undefined,
  );

  if (!account || !txsCount) return;

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
