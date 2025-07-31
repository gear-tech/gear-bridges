import { useQuery } from '@tanstack/react-query';
import request from 'graphql-request';

import { INDEXER_ADDRESS } from '../consts';
import { graphql } from '../graphql';

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
  return useQuery({
    queryKey: ['transaction', id],
    queryFn: () => request(INDEXER_ADDRESS, TRANSFER_QUERY, { id }),
    select: (data) => data.transferById,
  });
}

export { useTransaction };
