import { useQuery } from '@tanstack/react-query';
import { request } from 'graphql-request';

import { useNetworkType } from '@/context';

import { graphql } from '../graphql';
import { TransferFilter } from '../types';

const TRANSFERS_COUNT_QUERY = graphql(`
  query TransfersCountQuery($filter: TransferFilter) {
    allTransfers(filter: $filter) {
      totalCount
    }
  }
`);

type Params = {
  filter?: TransferFilter;
  refetchInterval?: number;
  enabled?: boolean;
};

function useTransactionsCount({ filter, refetchInterval, enabled }: Params = {}) {
  const { NETWORK_PRESET } = useNetworkType();

  return useQuery({
    queryKey: ['transactionsCount', NETWORK_PRESET.INDEXER_ADDRESS, filter],

    queryFn: () =>
      request(NETWORK_PRESET.INDEXER_ADDRESS, TRANSFERS_COUNT_QUERY, {
        // assertion because postgraphile throws error on null or empty objects,
        // but we can't use undefined because graphlq-request requires exact arguments
        filter: filter!,
      }),

    select: (data) => data?.allTransfers?.totalCount || 0,
    enabled: enabled ?? true,
    refetchInterval,
  });
}

export { useTransactionsCount };
