import { useQuery } from '@tanstack/react-query';
import { request } from 'graphql-request';

import { INDEXER_ADDRESS } from '../consts';
import { graphql } from '../graphql';
import { TransferFilter } from '../types';

const TRANSFERS_COUNT_QUERY = graphql(`
  query TransfersCountQuery($filter: TransferFilter) {
    allTransfers(filter: $filter) {
      totalCount
    }
  }
`);

function useTransactionsCount(filter?: TransferFilter, refetchInterval?: number) {
  return useQuery({
    queryKey: ['transactionsCount', filter],

    queryFn: () =>
      request(INDEXER_ADDRESS, TRANSFERS_COUNT_QUERY, {
        // assertion because postgraphile throws error on null or empty objects,
        // but we can't use undefined because graphlq-request requires exact arguments
        filter: filter!,
      }),

    select: (data) => data?.allTransfers?.totalCount || 0,
    refetchInterval,
  });
}

export { useTransactionsCount };
