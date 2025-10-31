import { useQuery } from '@tanstack/react-query';
import { request } from 'graphql-request';

import { useNetworkType } from '@/context';
import { graphql } from '@/features/history/graphql';

const MERKLE_ROOT_IN_MESSAGE_QUEUES_QUERY = graphql(`
  query MerkelRootInMessageQueuesQuery($blockNumber: BigInt!) {
    allMerkleRootInMessageQueues(filter: { blockNumber: { greaterThanOrEqualTo: $blockNumber } }) {
      totalCount
    }
  }
`);

function useIsVaraRelayAvailable(blockNumber: string) {
  const { NETWORK_PRESET } = useNetworkType();

  return useQuery({
    queryKey: ['isVaraRelayAvailable', NETWORK_PRESET.INDEXER_ADDRESS, blockNumber],
    queryFn: () => request(NETWORK_PRESET.INDEXER_ADDRESS, MERKLE_ROOT_IN_MESSAGE_QUEUES_QUERY, { blockNumber }),
    select: (data) => Boolean(data.allMerkleRootInMessageQueues?.totalCount),
  });
}

export { useIsVaraRelayAvailable };
