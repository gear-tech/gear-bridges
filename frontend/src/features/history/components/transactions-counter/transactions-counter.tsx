import { Skeleton } from '@/components';

import { useTransactionsCount } from '../../hooks';
import { useTransactionsCountSubscription } from '../../hooks/use-transactions-count-subscription';

import styles from './transactions-counter.module.scss';

function TransactionsCounter() {
  const { data, isLoading } = useTransactionsCount();
  useTransactionsCountSubscription();

  return (
    <div>
      <p className={styles.text}>{isLoading ? <Skeleton width="64px" /> : data}</p>
      <h3 className={styles.heading}>Transactions (All time)</h3>
    </div>
  );
}

export { TransactionsCounter };
