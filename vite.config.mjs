import { defineConfig } from 'vite';
import react from '@vitejs/plugin-react';
import { resolve } from 'node:path';
const roots = { desktop: 'frontend', mobile: 'mobile', legacy: 'xiaomi_dlna/static' };
export default defineConfig(({ mode }) => {
  const surface = mode in roots ? mode : 'desktop';
  return {
    root: resolve(roots[surface]),
    base: './',
    plugins: [react()],
    server: {
      host: '127.0.0.1',
      port: surface === 'desktop' ? 1420 : surface === 'mobile' ? 1421 : 1422,
      strictPort: true,
      fs: { allow: [process.cwd()] },
    },
    build: {
      outDir: resolve(surface === 'legacy' ? 'xiaomi_dlna/static/dist' : `dist/${surface}`),
      emptyOutDir: true,
      cssCodeSplit: false,
      rollupOptions: {
        output: {
          codeSplitting: false,
          entryFileNames: 'app.js',
          assetFileNames: (asset) =>
            asset.names?.some((name) => name.endsWith('.css'))
              ? 'app.css'
              : 'assets/[name]-[hash][extname]',
        },
      },
    },
  };
});
