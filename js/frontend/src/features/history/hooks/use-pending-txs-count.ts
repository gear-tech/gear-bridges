import { useQueryClient } from '@tanstack/react-query';

import { TransfersCountQueryQuery } from '../graphql/graphql';
import { Status, TransferFilter } from '../types';

import { useAccountsFilter } from './use-accounts-filter';
import { useTransactionsCountQueryKey, useTransactionsCount } from './use-transactions-count';

function usePendingTxsCountFilter() {
  const { addresses, isAvailable } = useAccountsFilter();

  return {
    filter: { sender: { in: addresses }, status: { equalTo: Status.AwaitingPayment } } as TransferFilter,
    enabled: isAvailable,
  };
}

function useOptimisticPendingTxsCountUpdate() {
  const queryClient = useQueryClient();
  const { filter } = usePendingTxsCountFilter();
  const queryKey = useTransactionsCountQueryKey(filter);

  return () =>
    queryClient.setQueryData<TransfersCountQueryQuery>(queryKey, (data) => ({
      allTransfers: { totalCount: (data?.allTransfers?.totalCount || 0) + 1 },
    }));
}

function usePendingTxsCount() {
  const { filter, enabled } = usePendingTxsCountFilter();

  return useTransactionsCount({ filter, enabled, refetchInterval: 60000 });
}

export { usePendingTxsCount, useOptimisticPendingTxsCountUpdate };
