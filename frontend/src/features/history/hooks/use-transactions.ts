import { useInfiniteQuery } from '@tanstack/react-query';
import request from 'graphql-request';

import { isUndefined } from '@/utils';

import { INDEXER_ADDRESS, TELEPORTS_QUERY, TRANSACTIONS_LIMIT } from '../consts';
import { TeleportWhereInput, TeleportsQueryQuery } from '../graphql/graphql';

function useTransactions(transactionsCount: number | undefined, filters: TeleportWhereInput) {
  const isTransactionsCount = !isUndefined(transactionsCount);

  const getNextPageParam = (lastPage: TeleportsQueryQuery, allPages: TeleportsQueryQuery[]) => {
    if (!isTransactionsCount) throw new Error('Transactions count is not defined');

    const lastPageCount = lastPage.teleports.length;
    const fetchedCount = (allPages.length - 1) * TRANSACTIONS_LIMIT + lastPageCount;

    return fetchedCount < transactionsCount ? fetchedCount : undefined;
  };

  const { data, fetchNextPage, isFetching, hasNextPage } = useInfiniteQuery({
    queryKey: ['transactions', filters],
    queryFn: ({ pageParam }) =>
      request(INDEXER_ADDRESS, TELEPORTS_QUERY, { limit: TRANSACTIONS_LIMIT, offset: pageParam, where: filters }),
    initialPageParam: 0,
    getNextPageParam,
    enabled: isTransactionsCount,
  });

  const transactions = data?.pages.flatMap((page) => page.teleports);

  return [transactions, isFetching, hasNextPage, fetchNextPage] as const;
}

export { useTransactions };
