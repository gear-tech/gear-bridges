import { gql, makeExtendSchemaPlugin } from 'postgraphile';
import { PubSub } from 'graphql-subscriptions';
import pg from 'pg';

export const TransferCountSubscriptionPlugin = async (dbPool: pg.Pool) => {
  const COUNT_TOPIC = 'transfer_count';
  const pubsub = new PubSub();
  let count = 0;

  const client = await dbPool.connect();
  await client.query('LISTEN transfers_changed');

  client.on('notification', async () => {
    const result = await client.query('SELECT COUNT(*) FROM transfer');

    count = parseInt(result.rows[0].count, 10);

    pubsub.publish(COUNT_TOPIC, { transferCount: count });
  });

  return makeExtendSchemaPlugin(() => ({
    typeDefs: gql`
      extend type Subscription {
        transferCount: Int!
      }
    `,

    resolvers: {
      Subscription: {
        transferCount: {
          subscribe: () => pubsub.asyncIterator(COUNT_TOPIC),
          resolve: (_: any) => count,
        },
      },
    },
  }));
};
