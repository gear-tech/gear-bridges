import { Skeleton } from '@/components';

import { useTransactionsCount } from '../../hooks';

import styles from './transactions-counter.module.scss';

function TransactionsCounter() {
  const [count, isCountLoading] = useTransactionsCount();

  return (
    <div>
      <p className={styles.text}>{isCountLoading ? <Skeleton width="64px" /> : count}</p>
      <h3 className={styles.heading}>Transactions (All time)</h3>
    </div>
  );
}

export { TransactionsCounter };
