import { defineConfig } from 'vitest/config';
import react from '@vitejs/plugin-react';
export default defineConfig({
  plugins: [react()],
  test: {
    environment: 'jsdom',
    setupFiles: ['tests/react/setup.js'],
    include: ['tests/react/**/*.test.jsx'],
    maxWorkers: 2,
    restoreMocks: true,
    clearMocks: true,
  },
});
