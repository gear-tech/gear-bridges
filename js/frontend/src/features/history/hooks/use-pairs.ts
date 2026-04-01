import { HexString } from '@gear-js/api';
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
const OLD_VARA_TOKEN = '0xdbf80fe5bd78b44510762770a14dc2a5b13a6bb167ff12c2edcc7ca3deadc16d';
const NEW_VARA_TOKEN = '0x29c42c668012b1ce20720e4615229215023281ef4676fdc77bf047d7fbcb9d17';

const derivePairs = ({ allPairs }: PairsQueryQuery) => {
  const pairs = allPairs?.nodes as (Pair & { varaToken: HexString; ethToken: HexString })[];

  return pairs?.map((pair) => (pair.varaToken === OLD_VARA_TOKEN ? { ...pair, varaToken: NEW_VARA_TOKEN } : pair));
};

function usePairs() {
  const { NETWORK_PRESET } = useNetworkType();

  return useQuery({
    queryKey: ['pairs', NETWORK_PRESET.INDEXER_ADDRESS],
    queryFn: () => request(NETWORK_PRESET.INDEXER_ADDRESS, PAIRS_QUERY),
    select: derivePairs,
  });
}

export { usePairs };
