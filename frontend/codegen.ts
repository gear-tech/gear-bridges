import { CodegenConfig } from '@graphql-codegen/cli';
import { loadEnv } from 'vite';

const config: CodegenConfig = {
  schema: loadEnv('', process.cwd(), '').VITE_INDEXER_ADDRESS,
  documents: ['src/**/*.{ts,tsx}'],
  ignoreNoDocuments: true, // for better experience with the watcher
  generates: {
    './src/features/history/graphql/': {
      preset: 'client',
      plugins: [],
      config: {
        scalars: {
          DateTime: 'string', // custom subsquid scalars
          BigInt: 'string',
        },
        avoidOptionals: true,
      },
    },
  },
};

export default config;
