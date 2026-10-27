import js from '@eslint/js';
import globals from 'globals';
import react from 'eslint-plugin-react';
import hooks from 'eslint-plugin-react-hooks';
export default [
  { ignores: ['**/dist/**', '**/target/**', '**/vendor/**', '.tools/**', 'node_modules/**'] },
  {
    files: [
      'frontend/src/**/*.{js,jsx}',
      'mobile/src/**/*.{js,jsx}',
      'xiaomi_dlna/static/src/**/*.{js,jsx}',
    ],
    languageOptions: {
      ecmaVersion: 'latest',
      sourceType: 'module',
      parserOptions: { ecmaFeatures: { jsx: true } },
      globals: globals.browser,
    },
    plugins: { react, 'react-hooks': hooks },
    rules: {
      'no-undef': js.configs.recommended.rules['no-undef'],
      'react/jsx-key': 'error',
      'react/jsx-no-duplicate-props': 'error',
      'react-hooks/rules-of-hooks': 'error',
    },
  },
];
