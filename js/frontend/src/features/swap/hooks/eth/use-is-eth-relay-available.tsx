import { getSlotByBlockNumber } from '@gear-js/bridge';
import { useQuery } from '@tanstack/react-query';
import { request } from 'graphql-request';
import { usePublicClient } from 'wagmi';

import { INDEXER_ADDRESS } from '@/features/history/consts';
import { graphql } from '@/features/history/graphql';

import { ETH_BEACON_NODE_ADDRESS } from '../../consts';

const CHECKPOINT_SLOTS_QUERY = graphql(`
  query CheckpointSlotsQuery($slot: BigInt!) {
    allCheckpointSlots(filter: { slot: { greaterThanOrEqualTo: $slot } }) {
      totalCount
    }
  }
`);

function useIsEthRelayAvailable(blockNumber: bigint) {
  const publicClient = usePublicClient();

  const { data: slot } = useQuery({
    queryKey: ['slotByBlockNumber', blockNumber.toString()],
    queryFn: () => getSlotByBlockNumber(ETH_BEACON_NODE_ADDRESS, publicClient!, blockNumber),
    enabled: Boolean(publicClient),
    select: (data) => data?.toString(),
  });

  return useQuery({
    queryKey: ['isEthRelayAvailable', slot],
    queryFn: () => request(INDEXER_ADDRESS, CHECKPOINT_SLOTS_QUERY, { slot: slot! }),
    enabled: Boolean(slot),
    select: (data) => Boolean(data.allCheckpointSlots?.totalCount),
  });
}

export { useIsEthRelayAvailable };
