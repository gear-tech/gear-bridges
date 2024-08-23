import { useApi } from '@gear-js/react-hooks';
import { useQuery } from '@tanstack/react-query';

import { BALANCE_REFETCH_INTERVAL } from '../../consts';

function useDeriveBalancesAll(accountAddress: string | undefined) {
  const { api, isApiReady } = useApi();

  const isEnabled = isApiReady && !!accountAddress;

  return useQuery({
    queryKey: ['deriveBalancesAll', isApiReady, accountAddress],
    queryFn: () => (isEnabled ? api.derive.balances.all(accountAddress) : null),
    enabled: isEnabled,
    refetchInterval: BALANCE_REFETCH_INTERVAL,
  });
}

export { useDeriveBalancesAll };
