import { HexString } from '@gear-js/api';
import { useQuery } from '@tanstack/react-query';
import { request } from 'graphql-request';

import { useNetworkType } from '@/context';

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

  return useQuery({
    queryKey: ['pairs', NETWORK_PRESET.INDEXER_ADDRESS],
    queryFn: () => request(NETWORK_PRESET.INDEXER_ADDRESS, PAIRS_QUERY),
    select: derivePairs,
  });
}

export { usePairs };
