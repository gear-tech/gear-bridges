import { getSlotByBlockNumber } from '@gear-js/bridge';
import { useAlert } from '@gear-js/react-hooks';
import { useQuery } from '@tanstack/react-query';
import { request } from 'graphql-request';
import { useEffect } from 'react';
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
  const alert = useAlert();
  const publicClient = usePublicClient();

  const { data: slot, error } = useQuery({
    queryKey: ['slotByBlockNumber', blockNumber.toString()],
    queryFn: () => getSlotByBlockNumber(ETH_BEACON_NODE_ADDRESS, publicClient!, blockNumber),
    enabled: Boolean(publicClient),
    select: (data) => data?.toString(),
  });

  useEffect(() => {
    if (!error) return;

    alert.error(error.message);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [error]);

  return useQuery({
    queryKey: ['isEthRelayAvailable', slot],
    queryFn: () => request(INDEXER_ADDRESS, CHECKPOINT_SLOTS_QUERY, { slot: slot! }),
    enabled: Boolean(slot),
    select: (data) => Boolean(data.allCheckpointSlots?.totalCount),
  });
}

export { useIsEthRelayAvailable };
