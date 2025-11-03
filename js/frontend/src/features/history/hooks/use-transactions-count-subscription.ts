import { useAlert } from '@gear-js/react-hooks';
import { useQueryClient } from '@tanstack/react-query';
import { createClient } from 'graphql-ws';
import { useEffect } from 'react';

import { useNetworkType } from '@/context/network-type';
import { logger } from '@/utils';

const TRANSFERS_COUNT_SUBSCRIPTION = `
  subscription TransfersCountSubscription {
    transferCount
  }
`;

function useTransactionsCountSubscription() {
  const alert = useAlert();
  const queryClient = useQueryClient();
  const { NETWORK_PRESET } = useNetworkType();

  useEffect(() => {
    const wsClient = createClient({ url: NETWORK_PRESET.INDEXER_ADDRESS });

    const unsubscribe = wsClient.subscribe(
      { query: TRANSFERS_COUNT_SUBSCRIPTION },
      {
        next: (result) => {
          queryClient.setQueryData(['transactionsCount', undefined], {
            allTransfers: { totalCount: result.data?.transferCount },
          });
        },
        error: (error: Error) => {
          logger.error('Transaction count subscription', error);
          alert.error('Failed to subscribe to transaction count updates');
        },
        complete: () => {},
      },
    );

    return () => {
      void wsClient.dispose();
      unsubscribe();
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [NETWORK_PRESET.INDEXER_ADDRESS]);
}

export { useTransactionsCountSubscription };
