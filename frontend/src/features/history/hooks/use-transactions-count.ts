// import { useQuery } from '@tanstack/react-query';
// import { request } from 'graphql-request';

// import { INDEXER_ADDRESS } from '../consts';
// import { TRANSFERS_CONNECTION_QUERY } from '../consts/queries';

function useTransactionsCount(_where: object | null = null) {
  // const { data, isLoading } = useQuery({
  //   queryKey: ['transactionsCount', where],
  //   queryFn: () => request(INDEXER_ADDRESS, TRANSFERS_CONNECTION_QUERY, { where }),
  //   refetchInterval: 10000,
  // });

  const data = { transfersConnection: { totalCount: 0 } };
  const isLoading = false;

  return [data?.transfersConnection.totalCount, isLoading] as const;
}

export { useTransactionsCount };
