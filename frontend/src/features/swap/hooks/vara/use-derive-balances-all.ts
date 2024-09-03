import { useApi } from '@gear-js/react-hooks';
import { useQuery } from '@tanstack/react-query';

import { BALANCE_REFETCH_INTERVAL } from '../../consts';

function useDeriveBalancesAll(accountAddress: string | undefined) {
  const { api, isApiReady } = useApi();

  const getDeriveBalancesAll = async () => {
    if (!isApiReady) throw new Error('API is not initialized');
    if (!accountAddress) throw new Error('Account is not found');

    return api.derive.balances.all(accountAddress);
  };

  return useQuery({
    queryKey: ['deriveBalancesAll', isApiReady, accountAddress],
    queryFn: getDeriveBalancesAll,
    enabled: isApiReady && Boolean(accountAddress),
    refetchInterval: BALANCE_REFETCH_INTERVAL,
  });
}

export { useDeriveBalancesAll };
