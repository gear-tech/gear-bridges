import { HexString } from '@gear-js/api';
import { useApi } from '@gear-js/react-hooks';
import { authenticateTokenPairs } from '@/context/token-identity';
import { useQuery } from '@tanstack/react-query';
import { request } from 'graphql-request';

import { useNetworkType } from '@/context/network-type';

import { graphql } from '../graphql';
import { Pair, PairsQueryQuery } from '../graphql/graphql';

const PAIRS_QUERY = graphql(`
  query PairsQuery {
    allPairs {
      nodes {
        ethToken
        ethTokenDecimals
        ethTokenName
        ethTokenSymbol
        id
        isActive
        tokenSupply
        varaToken
        varaTokenDecimals
        varaTokenName
        varaTokenSymbol
      }
    }
  }
`);

const derivePairs = ({ allPairs }: PairsQueryQuery) =>
  allPairs?.nodes as (Pair & { varaToken: HexString; ethToken: HexString })[];

function usePairs() {
  const { NETWORK_PRESET } = useNetworkType();

  const { api } = useApi();
  const profile = NETWORK_PRESET.INBOUND_PROOF_PROFILE.profile;
  const query = useQuery({
    queryKey: [
      'pairs',
      NETWORK_PRESET.INDEXER_ADDRESS,
      JSON.stringify(profile, (_, value: unknown) => (typeof value === 'bigint' ? value.toString() : value)),
      api?.genesisHash.toHex(),
    ],
    enabled: !!api && !!profile,
    queryFn: async ({ signal }) => {
      if (!api || !profile) throw new Error('HOLD: approved token profile unavailable');
      const started = Date.now();
      const result = await request(NETWORK_PRESET.INDEXER_ADDRESS, PAIRS_QUERY);
      const pairs = derivePairs(result);
      if (!Array.isArray(pairs)) throw new Error('HOLD: token inventory unavailable');
      await authenticateTokenPairs(api, profile, pairs);
      signal.throwIfAborted();
      if (Date.now() - started >= 30_000) throw new Error('HOLD: token qualification response is stale');
      return result;
    },
    select: derivePairs,
    staleTime: 0,
    refetchInterval: 30_000,
    refetchOnWindowFocus: 'always',
    refetchOnReconnect: 'always',
  });
  // A failed/in-flight refresh must not leave cached active signing capabilities enabled.
  return { ...query, data: query.isError || query.isFetching || !api || !profile ? undefined : query.data };
}

export { usePairs };
