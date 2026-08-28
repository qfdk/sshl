import { defineConfig } from 'vite';
import react from '@vitejs/plugin-react';
import tailwindcss from '@tailwindcss/vite';

export default defineConfig({
  root: 'ui',
  base: './',
  plugins: [react(), tailwindcss()],
  build: {
    outDir: 'dist',
  },
  server: {
    strictPort: true,
    port: 5173,
  },
});
