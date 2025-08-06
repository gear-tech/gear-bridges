import { Plugin, postgraphile, PostGraphileOptions } from 'postgraphile';
import ConnectionFilterPlugin from 'postgraphile-plugin-connection-filter';
import { createServer } from 'node:http';
import express from 'express';
import dotenv from 'dotenv';
import { Pool } from 'pg';
import { DefaultLimitPlugin, PairsAliasPlugin, TransferAliasPlugin, TransferCountSubscriptionPlugin } from './plugins';

dotenv.config();

const isDev = process.env.NODE_ENV === 'development';

async function main() {
  const dbPool = new Pool({ connectionString: process.env.DATABASE_URL || 'postgres://bridge' });

  const options: PostGraphileOptions = {
    watchPg: isDev,
    graphiql: true,
    enhanceGraphiql: isDev,
    subscriptions: true,
    dynamicJson: true,
    setofFunctionsContainNulls: false,
    disableDefaultMutations: true,
    ignoreRBAC: false,
    showErrorStack: isDev ? 'json' : true,
    extendedErrors: ['hint', 'detail', 'errcode'],
    allowExplain: isDev,
    legacyRelations: 'omit',
    exportGqlSchemaPath: `${__dirname}/schema.graphql`,
    sortExport: true,
    appendPlugins: [
      await TransferCountSubscriptionPlugin(dbPool),
      DefaultLimitPlugin(50, 100),
      PairsAliasPlugin,
      TransferAliasPlugin,
      // TODO: get rid of this plugin
      ConnectionFilterPlugin as unknown as Plugin,
    ],
  };

  const middleware = postgraphile(dbPool, 'public', options);

  const app = express();

  app.use(middleware);

  const server = createServer(app);

  const port = process.env.GQL_PORT || 4350;

  server.listen({ host: '0.0.0.0', port }, () => {
    const address = server.address()!;
    if (typeof address !== 'string') {
      const href = `http://${address.address}:${address.port}${options.graphiqlRoute || '/graphiql'}`;
      console.log(`PostGraphiQL available at ${href} 🚀`);
    } else {
      console.log(`PostGraphile listening on ${address} 🚀`);
    }
  });
}

main().catch((error) => {
  console.error(error);
  process.exit(1);
});
