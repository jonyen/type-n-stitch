import react from '@vitejs/plugin-react';
import { defineConfig } from 'vite';

const server = 'http://localhost:5175';

export default defineConfig({
  plugins: [react()],
  server: {
    port: 5174,
    strictPort: true,
    proxy: {
      '/api': { target: server, ws: true },
      '/data': server,
      '/library': server,
      '/fonts': server,
      '/mcp': { target: server, changeOrigin: true },
    },
  },
});
