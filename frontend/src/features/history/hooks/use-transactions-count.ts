import { useQuery } from '@tanstack/react-query';
import request from 'graphql-request';

import { INDEXER_ADDRESS } from '../consts';
import { TELEPORTS_CONNECTION_QUERY } from '../consts/queries';
import { TeleportWhereInput } from '../graphql/graphql';

function useTransactionsCount(where: TeleportWhereInput | null = null) {
  const { data, isLoading } = useQuery({
    queryKey: ['transactionsCount', where],
    queryFn: () => request(INDEXER_ADDRESS, TELEPORTS_CONNECTION_QUERY, { where }),
  });

  return [data?.teleportsConnection.totalCount, isLoading] as const;
}

export { useTransactionsCount };
