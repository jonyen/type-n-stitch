import react from '@vitejs/plugin-react';
import { defineConfig } from 'vitest/config';

export default defineConfig({
  plugins: [react()],
  test: {
    include: ['client/src/**/*.test.{ts,tsx}'],
    setupFiles: ['client/src/test/setup.ts'],
  },
});
