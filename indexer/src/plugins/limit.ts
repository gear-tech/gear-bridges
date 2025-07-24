import { GraphQLObjectType } from 'graphql';
import { Plugin } from 'postgraphile';

export function DefaultLimitPlugin(limit = 50, maxLimit = 1000): Plugin {
  return (builder) => {
    builder.hook('GraphQLObjectType:fields', (fields, build, { scope }) => {
      const isConnectionLike = scope.isRootQuery;

      if (!isConnectionLike) return fields;

      return build.extend(
        fields,
        Object.fromEntries(
          Object.entries(fields).map(([fieldName, field]) => {
            const origResolve = field.resolve;
            if (!origResolve) return [fieldName, field];

            const returnType = field.type;

            const isConnectionLike =
              returnType instanceof GraphQLObjectType &&
              returnType.getFields()['pageInfo'] &&
              returnType.getFields()['edges'];

            if (!isConnectionLike) return [fieldName, field];

            field.resolve = (parent, args, ctx, info) => {
              if (args.first == null && args.last == null) {
                args = { ...args, first: limit };
              }

              if (args.first != null && args.first > maxLimit) {
                args.first = maxLimit;
              }

              if (args.last != null && args.last > maxLimit) {
                args.last = maxLimit;
              }

              return origResolve(parent, args, ctx, info);
            };

            return [fieldName, field];
          }),
        ),
      );
    });
  };
}
