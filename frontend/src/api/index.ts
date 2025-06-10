import { HexString } from '@gear-js/api';
import { useQuery } from '@tanstack/react-query';
import { request } from 'graphql-request';

import { INDEXER_ADDRESS } from './consts';
import { graphql } from './graphql';
import { Pair } from './graphql/graphql';

const PAIRS_QUERY = graphql(`
  query PairsQuery {
    pairs {
      ethToken
      ethTokenDecimals
      ethTokenName
      ethTokenSymbol
      id
      isRemoved
      tokenSupply
      varaToken
      varaTokenDecimals
      varaTokenName
      varaTokenSymbol
    }
  }
`);

function usePairs() {
  return useQuery({
    queryKey: ['pairs'],
    queryFn: () => request(INDEXER_ADDRESS, PAIRS_QUERY),
    select: ({ pairs }) => pairs as (Pair & { varaToken: HexString; ethToken: HexString })[],
  });
}

export { usePairs };
