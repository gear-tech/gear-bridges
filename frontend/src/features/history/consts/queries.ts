import { graphql } from '../graphql';

const TELEPORTS_QUERY = graphql(`
  query TeleportsQuery($limit: Int!, $offset: Int!, $where: TeleportWhereInput) {
    teleports(limit: $limit, offset: $offset, orderBy: timestamp_DESC, where: $where) {
      amount
      blockhash
      direction
      from
      id
      status
      timestamp
      to
      pair
    }
  }
`);

const TELEPORTS_CONNECTION_QUERY = graphql(`
  query TeleportsConnectionQuery($where: TeleportWhereInput) {
    teleportsConnection(orderBy: timestamp_DESC, where: $where) {
      totalCount
    }
  }
`);

export { TELEPORTS_QUERY, TELEPORTS_CONNECTION_QUERY };
