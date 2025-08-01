import { graphql } from '../graphql';

const TRANSACTIONS_LIMIT = 12;

const TRANSFERS_QUERY = graphql(`
  query TransfersQuery($first: Int!, $offset: Int!, $filter: TransferFilter) {
    allTransfers(first: $first, offset: $offset, orderBy: TIMESTAMP_DESC, filter: $filter) {
      nodes {
        amount
        txHash
        destNetwork
        destination
        id
        receiver
        sender
        source
        sourceNetwork
        status
        timestamp
        nonce
        blockNumber
      }

      totalCount
    }
  }
`);

export { TRANSACTIONS_LIMIT, TRANSFERS_QUERY };
