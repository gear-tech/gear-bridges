import { useAccount } from '@gear-js/react-hooks';
import { useQueryClient } from '@tanstack/react-query';

import { useEthAccount } from '@/hooks';

import { TransfersCountQueryQuery } from '../graphql/graphql';
import { Status, TransferFilter } from '../types';

import { useTransactionsCountQueryKey, useTransactionsCount } from './use-transactions-count';

function usePendingTxsCountFilter() {
  const { account } = useAccount();
  const ethAccount = useEthAccount();

  const accountAddress = account?.decodedAddress || ethAccount.address?.toLowerCase();

  const filter = { sender: { equalTo: accountAddress }, status: { equalTo: Status.AwaitingPayment } } as TransferFilter;
  const enabled = Boolean(accountAddress);

  return { filter, enabled };
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
