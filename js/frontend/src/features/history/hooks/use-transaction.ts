import { useQuery, useQueryClient } from '@tanstack/react-query';
import { request } from 'graphql-request';

import { useNetworkType } from '@/context';

import { graphql } from '../graphql';
import { StatusEnum, TransferQueryQuery } from '../graphql/graphql';

const TRANSFER_QUERY = graphql(`
  query TransferQuery($id: String!) {
    transferById(id: $id) {
      id
      txHash
      blockNumber
      timestamp
      completedAt
      completedAtBlock
      completedAtTxHash
      nonce
      sourceNetwork
      source
      destNetwork
      destination
      status
      sender
      receiver
      amount
      bridgingStartedAtBlock
      bridgingStartedAtMessageId
    }
  }
`);

function useTransaction(id: string) {
  const { NETWORK_PRESET } = useNetworkType();

  return useQuery({
    queryKey: ['transaction', NETWORK_PRESET.INDEXER_ADDRESS, id],
    queryFn: () => request(NETWORK_PRESET.INDEXER_ADDRESS, TRANSFER_QUERY, { id }),
    select: (data) => data.transferById,
  });
}

function useOptimisticTxUpdate(id: string) {
  const queryClient = useQueryClient();

  return (status = StatusEnum.Completed) =>
    queryClient.setQueryData<TransferQueryQuery>(['transaction', id], (data) => {
      if (!data?.transferById) return data;

      return { transferById: { ...data.transferById, status } };
    });
}

export { useTransaction, useOptimisticTxUpdate };
