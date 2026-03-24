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

const OLD_VARA_TOKEN = '0xd0f89cfd994c92bb743a5a69049609b796e2026e05318f7eef621a5e31df3d4b';
const NEW_VARA_TOKEN = '0xa1a37e5a36e8a53921f6bedefadec91dc510636079a22238e9edf8233aaa494e';

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
