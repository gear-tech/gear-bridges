import { useInfiniteQuery } from '@tanstack/react-query';
import { request } from 'graphql-request';

import { isUndefined } from '@/utils';

import { INDEXER_ADDRESS, TRANSFERS_QUERY, TRANSACTIONS_LIMIT } from '../consts';
import { TransferFilter, TransfersQueryQuery } from '../graphql/graphql';

function useTransactions(transactionsCount: number | undefined, filter: TransferFilter | undefined) {
  const isTransactionsCount = !isUndefined(transactionsCount);

  const getNextPageParam = (lastPage: TransfersQueryQuery, allPages: TransfersQueryQuery[]) => {
    if (!isTransactionsCount) throw new Error('Transactions count is not defined');

    const lastPageCount = lastPage.allTransfers?.nodes.length || 0;
    const fetchedCount = (allPages.length - 1) * TRANSACTIONS_LIMIT + lastPageCount;

    return fetchedCount < transactionsCount ? fetchedCount : undefined;
  };

  const { data, fetchNextPage, isFetching, hasNextPage } = useInfiniteQuery({
    queryKey: ['transactions', filter],

    queryFn: ({ pageParam }) =>
      request(INDEXER_ADDRESS, TRANSFERS_QUERY, {
        first: TRANSACTIONS_LIMIT,
        offset: pageParam,

        // assertion because postgraphile throws error on null,
        // but we can't use undefined because graphlq-request requires exact arguments
        filter: filter as TransferFilter,
      }),

    initialPageParam: 0,
    getNextPageParam,
    enabled: isTransactionsCount,
    select: ({ pages }) => pages.flatMap(({ allTransfers }) => allTransfers?.nodes || []),
  });

  return [data, isFetching, hasNextPage, fetchNextPage] as const;
}

export { useTransactions };
