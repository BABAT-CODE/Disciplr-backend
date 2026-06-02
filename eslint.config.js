import js from '@eslint/js'
import tseslint from '@typescript-eslint/eslint-plugin'
import tsParser from '@typescript-eslint/parser'

export default [
  js.configs.recommended,
  {
    files: ['src/**/*.ts'],
    languageOptions: {
      parser: tsParser,
      parserOptions: {
        ecmaVersion: 'latest',
        sourceType: 'module',
      },
      globals: {
        console: 'readonly',
        process: 'readonly',
      },
    },
    plugins: {
      '@typescript-eslint': tseslint,
    },
    rules: {
      ...tseslint.configs.recommended.rules,
      'no-restricted-properties': [
        'warn',
        {
          object: 'process',
          property: 'env',
          message: 'Use getEnv() from src/config/env.ts instead of accessing process.env directly.',
        },
      ],
    },
  },
  {
    files: ['src/config/env.ts'],
    rules: {
      'no-restricted-properties': 'off',
    },
  },
]
