import { useAccount } from '@gear-js/react-hooks';

import { useEthAccount } from '@/hooks';

import { Status, TransferFilter } from '../types';

import { useTransactionsCount } from './use-transactions-count';

function usePendingTxsCount() {
  const { account } = useAccount();
  const ethAccount = useEthAccount();

  const accountAddress = account?.decodedAddress || ethAccount.address?.toLowerCase();
  const filter = { sender: { equalTo: accountAddress }, status: { equalTo: Status.AwaitingPayment } } as TransferFilter;

  return useTransactionsCount({ filter, refetchInterval: 60000, enabled: Boolean(accountAddress) });
}

export { usePendingTxsCount };
