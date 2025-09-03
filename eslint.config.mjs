import js from '@eslint/js';
import tseslint from 'typescript-eslint';
import tsParser from '@typescript-eslint/parser';
import tsPlugin from '@typescript-eslint/eslint-plugin';
import importPlugin from 'eslint-plugin-import';
import prettierConfig from 'eslint-config-prettier';

const files = ['js/bridge-js/{src,test,example}/**/*.ts', 'js/indexer/src/**/*.ts', 'js/common/src/**/*.ts'];
const project = ['./js/bridge-js/tsconfig.json', './js/indexer/tsconfig.json', './js/common/tsconfig.json'];
const noUnusedVars = [
  'error',
  {
    args: 'all',
    argsIgnorePattern: '^_',
    varsIgnorePattern: '^_',
    caughtErrorsIgnorePattern: '^_',
    destructuredArrayIgnorePattern: '^_',
    ignoreRestSiblings: true,
  },
];

export default [
  {
    ignores: ['node_modules/**', '**/dist/**', '**/lib/**', 'js/frontend/**', '*.js'],
  },
  ...[js.configs.recommended, ...tseslint.configs.recommended].map((cfg) => ({ ...cfg, files })),
  {
    files,
    languageOptions: {
      parser: tsParser,
      parserOptions: {
        ecmaVersion: 2022,
        sourceType: 'module',
        project,
      },
      globals: {
        node: true,
        es2022: true,
      },
    },
    plugins: {
      '@typescript-eslint': tsPlugin,
      import: importPlugin,
    },
    rules: {
      '@typescript-eslint/no-unused-vars': noUnusedVars,
      '@typescript-eslint/no-explicit-any': 'off',
      'prefer-rest-params': 'off',
      '@typescript-eslint/no-empty-object-type': ['error', { allowInterfaces: 'with-single-extends' }],
    },
  },
  prettierConfig,
];
