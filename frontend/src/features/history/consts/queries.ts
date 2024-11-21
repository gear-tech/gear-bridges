import { graphql } from '../graphql';

const LATEST_TRANSACTIONS_LIMIT = 5;
const TRANSACTIONS_LIMIT = 12;

const TRANSFERS_QUERY = graphql(`
  query TransfersQuery($limit: Int!, $offset: Int!, $where: TransferWhereInput) {
    transfers(limit: $limit, offset: $offset, orderBy: timestamp_DESC, where: $where) {
      amount
      blockNumber
      destNetwork
      destination
      id
      receiver
      sender
      source
      sourceNetwork
      status
      timestamp
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

export { TRANSACTIONS_LIMIT, LATEST_TRANSACTIONS_LIMIT, TRANSFERS_QUERY, TRANSFERS_CONNECTION_QUERY };
