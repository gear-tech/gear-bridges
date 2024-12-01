import { useQuery } from '@tanstack/react-query';
import { request } from 'graphql-request';
import { useMemo } from 'react';

import { useInvalidateOnBlock, useTokens } from '@/hooks';

import { TRANSFERS_QUERY, LATEST_TRANSACTIONS_LIMIT, INDEXER_ADDRESS } from '../../consts';
import { TransactionCard } from '../transaction-card';

import styles from './latest-transactions.module.scss';

function useLatestTransactions() {
  const queryKey = useMemo(() => ['latestTransactions'], []);

  const query = useQuery({
    queryKey,
    queryFn: () =>
      request(INDEXER_ADDRESS, TRANSFERS_QUERY, { limit: LATEST_TRANSACTIONS_LIMIT, offset: 0, where: null }),
    select: ({ transfers }) => transfers,
  });

  useInvalidateOnBlock({ queryKey });

  return query;
}

function LatestTransactions() {
  const { data, isLoading: isTransactionsQueryLoading } = useLatestTransactions();
  const { decimals, symbols, isLoading: isTokensQueryLoading } = useTokens();

  const isLoading = isTransactionsQueryLoading || isTokensQueryLoading;

  const renderTransactions = () =>
    decimals &&
    symbols &&
    data?.map((transaction) => (
      <li key={transaction.id}>
        <TransactionCard.Compact {...transaction} decimals={decimals} symbols={symbols} />
      </li>
    ));

  const renderSkeletons = () =>
    new Array(LATEST_TRANSACTIONS_LIMIT).fill(null).map((_, index) => (
      <li key={index}>
        <TransactionCard.Skeleton isCompact />
      </li>
    ));

  if (!isLoading && !data?.length) return <p className={styles.text}>No transactions found at the moment.</p>;

  return <ul className={styles.list}>{isLoading ? renderSkeletons() : renderTransactions()}</ul>;
}

export { LatestTransactions };
