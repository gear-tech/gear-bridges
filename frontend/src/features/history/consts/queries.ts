import { graphql } from '../graphql';

const TRANSACTIONS_LIMIT = 12;

const TRANSFERS_QUERY = graphql(`
  query TransfersQuery($limit: Int!, $offset: Int!, $where: TransferWhereInput) {
    transfers(limit: $limit, offset: $offset, orderBy: timestamp_DESC, where: $where) {
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
  }
`);

const TRANSFERS_CONNECTION_QUERY = graphql(`
  query TransfersConnectionQuery($where: TransferWhereInput) {
    transfersConnection(orderBy: timestamp_DESC, where: $where) {
      totalCount
    }
  }
`);

export { TRANSACTIONS_LIMIT, TRANSFERS_QUERY, TRANSFERS_CONNECTION_QUERY };
