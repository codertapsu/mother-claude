// @ts-check
// ESLint for the browser extension: the repository's rule sets (eslint and
// typescript-eslint recommended + stylistic, as in ../eslint.config.js) plus
// type-aware checks for floating promises, which matter in a service worker.
import eslint from '@eslint/js';
import { defineConfig, globalIgnores } from 'eslint/config';
import tseslint from 'typescript-eslint';

export default defineConfig([
  globalIgnores(['**/node_modules/**']),
  {
    files: ['**/*.ts'],
    extends: [eslint.configs.recommended, tseslint.configs.recommended, tseslint.configs.stylistic],
    languageOptions: {
      parserOptions: {
        projectService: true,
        tsconfigRootDir: import.meta.dirname,
      },
    },
    rules: {
      '@typescript-eslint/no-floating-promises': 'error',
      '@typescript-eslint/no-misused-promises': [
        'error',
        { checksVoidReturn: { arguments: false, attributes: false } },
      ],
      '@typescript-eslint/await-thenable': 'error',
      '@typescript-eslint/consistent-type-imports': ['error', { fixStyle: 'inline-type-imports' }],
      'no-restricted-globals': [
        'error',
        { name: 'process', message: 'Extension code runs in the browser.' },
        { name: 'Buffer', message: 'Extension code runs in the browser.' },
      ],
    },
  },
  {
    // Tests and the e2e harness run in Node; node:test's describe/it return promises by design.
    files: ['test/**/*.ts'],
    rules: {
      'no-restricted-globals': 'off',
      '@typescript-eslint/no-floating-promises': [
        'error',
        {
          allowForKnownSafeCalls: [
            {
              from: 'package',
              package: 'node:test',
              name: [
                'describe',
                'it',
                'test',
                'suite',
                'before',
                'after',
                'beforeEach',
                'afterEach',
              ],
            },
          ],
        },
      ],
    },
  },
  {
    files: ['**/*.mjs', '**/*.js'],
    extends: [eslint.configs.recommended],
    languageOptions: {
      globals: {
        process: 'readonly',
        console: 'readonly',
        Buffer: 'readonly',
        URL: 'readonly',
        structuredClone: 'readonly',
      },
    },
  },
]);
