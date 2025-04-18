import { useQuery } from '@tanstack/react-query';
import { request } from 'graphql-request';

import { INDEXER_ADDRESS } from '../consts';
import { TRANSFERS_CONNECTION_QUERY } from '../consts/queries';
import { TransferWhereInput } from '../graphql/graphql';

function useTransactionsCount(where: TransferWhereInput | null = null) {
  const { data, isLoading } = useQuery({
    queryKey: ['transactionsCount', where],
    queryFn: () => request(INDEXER_ADDRESS, TRANSFERS_CONNECTION_QUERY, { where }),
    refetchInterval: 10000,
  });

  return [data?.transfersConnection.totalCount, isLoading] as const;
}

export { useTransactionsCount };
