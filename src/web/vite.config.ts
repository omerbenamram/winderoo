import { defineConfig } from 'vite'
import preact from '@preact/preset-vite'
import compression from 'vite-plugin-compression'

export default defineConfig({
  plugins: [
    preact(),
    compression({
      algorithm: 'gzip',
      ext: '.gz',
      threshold: 0,
      deleteOriginFile: false,
    }),
  ],
  build: {
    outDir: 'dist',
    minify: 'esbuild',
    target: 'es2020',
    rollupOptions: {
      output: {
        // Single chunk for ESP32 simplicity
        manualChunks: undefined,
        entryFileNames: 'app.js',
        chunkFileNames: '[name].js',
        assetFileNames: (assetInfo) => {
          if (assetInfo.name?.endsWith('.css')) return 'app.css'
          return '[name][extname]'
        },
      },
    },
  },
})
