import { useQuery } from '@tanstack/react-query';
import request from 'graphql-request';

import { TELEPORTS_QUERY, REFETCH_INTERVAL, LATEST_TRANSACTIONS_LIMIT, INDEXER_ADDRESS } from '../../consts';
import { TransactionCard } from '../transaction-card';

import styles from './latest-transactions.module.scss';

function LatestTransactions() {
  const { data, isLoading } = useQuery({
    queryKey: ['latestTransactions'],
    queryFn: () =>
      request(INDEXER_ADDRESS, TELEPORTS_QUERY, { limit: LATEST_TRANSACTIONS_LIMIT, offset: 0, where: null }),
    refetchInterval: REFETCH_INTERVAL,
  });

  const transactions = data?.teleports;

  const renderTransactions = () =>
    transactions?.map((transaction) => (
      <li key={transaction.id}>
        <TransactionCard isCompact {...transaction} />
      </li>
    ));

  const renderSkeletons = () =>
    new Array(LATEST_TRANSACTIONS_LIMIT).fill(null).map((_, index) => (
      <li key={index}>
        <TransactionCard.Skeleton />
      </li>
    ));

  if (!isLoading && !transactions?.length) return <p className={styles.text}>No transactions found at the moment.</p>;

  return <ul className={styles.list}>{isLoading ? renderSkeletons() : renderTransactions()}</ul>;
}

export { LatestTransactions };
