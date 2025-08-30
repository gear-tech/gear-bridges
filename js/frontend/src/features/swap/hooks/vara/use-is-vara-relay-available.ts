import { useQuery } from '@tanstack/react-query';
import { request } from 'graphql-request';

import { INDEXER_ADDRESS } from '@/features/history/consts';
import { graphql } from '@/features/history/graphql';

const MERKLE_ROOT_IN_MESSAGE_QUEUES_QUERY = graphql(`
  query MerkelRootInMessageQueuesQuery($blockNumber: BigInt!) {
    allMerkleRootInMessageQueues(filter: { blockNumber: { greaterThanOrEqualTo: $blockNumber } }) {
      totalCount
    }
  }
`);

function useIsVaraRelayAvailable(blockNumber: string) {
  return useQuery({
    queryKey: ['isVaraRelayAvailable', blockNumber],
    queryFn: () => request(INDEXER_ADDRESS, MERKLE_ROOT_IN_MESSAGE_QUEUES_QUERY, { blockNumber }),
    select: (data) => Boolean(data.allMerkleRootInMessageQueues?.totalCount),
  });
}

export { useIsVaraRelayAvailable };
