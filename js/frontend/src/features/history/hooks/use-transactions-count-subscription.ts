import { useAlert } from '@gear-js/react-hooks';
import { useQueryClient } from '@tanstack/react-query';
import { createClient } from 'graphql-ws';
import { useEffect } from 'react';

import { useNetworkType } from '@/context/network-type';
import { logger } from '@/utils';

import { useTransactionsCountQueryKey } from './use-transactions-count';

const TRANSFERS_COUNT_SUBSCRIPTION = `
  subscription TransfersCountSubscription {
    transferCount
  }
`;

function useTransactionsCountSubscription() {
  const alert = useAlert();
  const queryClient = useQueryClient();
  const { NETWORK_PRESET } = useNetworkType();
  const queryKey = useTransactionsCountQueryKey();

  useEffect(() => {
    const wsClient = createClient({
      url: NETWORK_PRESET.INDEXER_ADDRESS.replace('http', 'ws'),
      shouldRetry: () => true,
    });

    const unsubscribe = wsClient.subscribe(
      { query: TRANSFERS_COUNT_SUBSCRIPTION },
      {
        next: (result) => {
          queryClient.setQueryData(queryKey, { allTransfers: { totalCount: result.data?.transferCount } });
        },
        error: (error: Error) => {
          logger.error('Transaction count subscription', error);
          alert.error('Failed to subscribe to transaction count updates');
        },
        complete: () => {},
      },
    );

    return () => {
      unsubscribe();
      void wsClient.dispose();
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [NETWORK_PRESET.INDEXER_ADDRESS, queryKey]);
}

export { useTransactionsCountSubscription };
