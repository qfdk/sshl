import { fileURLToPath, URL } from 'node:url';
import { defineConfig } from 'vite';
import react from '@vitejs/plugin-react';
import tailwindcss from '@tailwindcss/vite';

export default defineConfig({
  root: 'ui',
  resolve: {
    // tsconfig 的 paths 只作用于类型检查，Vite 运行时不读它，
    // 缺了这段 dev 模式下 shadcn 组件的 @/lib/utils 会解析失败。
    alias: {
      '@': fileURLToPath(new URL('./ui/src', import.meta.url)),
    },
  },
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
