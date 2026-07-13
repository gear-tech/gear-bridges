import { useQuery } from '@tanstack/react-query';

import { getCurrentTvl } from '../api';

const STALE_TIME = 5 * 60 * 1000;

function useTvl() {
  return useQuery({
    queryKey: ['tvl', 'vara-bridge'],
    queryFn: getCurrentTvl,
    staleTime: STALE_TIME,
    refetchInterval: STALE_TIME,
    retry: 2,
  });
}

export { useTvl };
