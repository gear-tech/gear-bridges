import { makeExtendSchemaPlugin, gql } from 'postgraphile';

export const PairsAliasPlugin = makeExtendSchemaPlugin(() => ({
  typeDefs: gql`
    extend type Query {
      pairs(first: Int, last: Int, offset: Int, orderBy: [PairsOrderBy!], filter: PairFilter): [Pair]!
    }
  `,
  resolvers: {
    Query: {
      pairs: async (_parent, _args, context, info) => {
        const result = await info.schema.getQueryType()!.getFields().allPairs.resolve!(_parent, _args, context, info);

        return result?.data ?? [];
      },
    },
  },
}));

export const TransferAliasPlugin = makeExtendSchemaPlugin(() => ({
  typeDefs: gql`
    extend type Query {
      transfers(first: Int, last: Int, offset: Int, orderBy: [TransfersOrderBy!], filter: TransferFilter): [Transfer]!
    }
  `,
  resolvers: {
    Query: {
      transfers: async (_parent, _args, context, info) => {
        const result = await info.schema.getQueryType()!.getFields().allTransfers.resolve!(
          _parent,
          _args,
          context,
          info,
        );

        return result?.data ?? [];
      },
    },
  },
}));
