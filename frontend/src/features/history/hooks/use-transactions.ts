import { useInfiniteQuery } from '@tanstack/react-query';
import { request } from 'graphql-request';

import { isUndefined } from '@/utils';

import { INDEXER_ADDRESS, TRANSFERS_QUERY, TRANSACTIONS_LIMIT } from '../consts';
import { TransferWhereInput, TransfersQueryQuery } from '../graphql/graphql';

function useTransactions(transactionsCount: number | undefined, filters: TransferWhereInput) {
  const isTransactionsCount = !isUndefined(transactionsCount);

  const getNextPageParam = (lastPage: TransfersQueryQuery, allPages: TransfersQueryQuery[]) => {
    if (!isTransactionsCount) throw new Error('Transactions count is not defined');

    const lastPageCount = lastPage.transfers.length;
    const fetchedCount = (allPages.length - 1) * TRANSACTIONS_LIMIT + lastPageCount;

    return fetchedCount < transactionsCount ? fetchedCount : undefined;
  };

  const { data, fetchNextPage, isFetching, hasNextPage } = useInfiniteQuery({
    queryKey: ['transactions', filters],
    queryFn: ({ pageParam }) =>
      request(INDEXER_ADDRESS, TRANSFERS_QUERY, { limit: TRANSACTIONS_LIMIT, offset: pageParam, where: filters }),
    initialPageParam: 0,
    getNextPageParam,
    enabled: isTransactionsCount,
    select: ({ pages }) => pages.flatMap(({ transfers }) => transfers),
  });

  return [data, isFetching, hasNextPage, fetchNextPage] as const;
}

export { useTransactions };
