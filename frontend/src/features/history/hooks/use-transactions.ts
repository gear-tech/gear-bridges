import { useInfiniteQuery } from '@tanstack/react-query';
import { request } from 'graphql-request';

import { INDEXER_ADDRESS, TRANSFERS_QUERY, TRANSACTIONS_LIMIT } from '../consts';
import { TransferFilter, TransfersQueryQuery } from '../graphql/graphql';

function useTransactions(filter: TransferFilter | undefined) {
  const getNextPageParam = (lastPage: TransfersQueryQuery, allPages: TransfersQueryQuery[]) => {
    const lastPageCount = lastPage.allTransfers?.nodes.length || 0;
    const fetchedCount = (allPages.length - 1) * TRANSACTIONS_LIMIT + lastPageCount;

    return fetchedCount < (lastPage.allTransfers?.totalCount || 0) ? fetchedCount : undefined;
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
    select: ({ pages }) => ({
      transactions: pages.flatMap(({ allTransfers }) => allTransfers?.nodes || []),
      transactionsCount: pages[0]?.allTransfers?.totalCount || 0,
    }),
  });

  return [data, isFetching, hasNextPage, fetchNextPage] as const;
}

export { useTransactions };
