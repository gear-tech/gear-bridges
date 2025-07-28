import { useAlert } from '@gear-js/react-hooks';
import { useQueryClient } from '@tanstack/react-query';
import { createClient } from 'graphql-ws';
import { useEffect } from 'react';

import { logger } from '@/utils';

import { INDEXER_ADDRESS } from '../consts';

const TRANSFERS_COUNT_SUBSCRIPTION = `
  subscription TransfersCountSubscription {
    transferCount
  }
`;

function useTransactionsCountSubscription() {
  const alert = useAlert();
  const queryClient = useQueryClient();

  useEffect(() => {
    const wsClient = createClient({ url: INDEXER_ADDRESS });

    const unsubscribe = wsClient.subscribe(
      { query: TRANSFERS_COUNT_SUBSCRIPTION },
      {
        next: (result) => queryClient.setQueryData(['transactionsCount', undefined], result.data?.transferCount || 0),
        error: (error: Error) => {
          logger.error('Transaction count subscription', error);
          alert.error('Failed to subscribe to transaction count updates');
        },
        complete: () => {},
      },
    );

    return () => {
      unsubscribe();
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);
}

export { useTransactionsCountSubscription };
