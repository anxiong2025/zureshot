import { defineConfig } from 'vite';
import { svelte } from '@sveltejs/vite-plugin-svelte';
import { resolve } from 'path';

// https://vitejs.dev/config/
export default defineConfig({
  plugins: [svelte()],
  clearScreen: false,
  server: {
    port: 5173,
    strictPort: true,
  },
  // Force browser exports for Svelte 5 — prevents mount() SSR error
  resolve: {
    conditions: ['browser', 'import', 'module', 'default'],
  },
  envPrefix: ['VITE_', 'TAURI_'],
  build: {
    target: ['es2021', 'chrome100', 'safari15'],
    minify: !process.env.TAURI_DEBUG ? 'esbuild' : false,
    sourcemap: !!process.env.TAURI_DEBUG,
    rollupOptions: {
      input: {
        main: resolve(__dirname, 'index.html'),
        thumbnail: resolve(__dirname, 'thumbnail.html'),
        'region-selector': resolve(__dirname, 'region-selector.html'),
        'recording-bar': resolve(__dirname, 'recording-bar.html'),
        'recording-overlay': resolve(__dirname, 'recording-overlay.html'),
        'camera-overlay': resolve(__dirname, 'camera-overlay.html'),
        countdown: resolve(__dirname, 'countdown.html'),
        'screenshot-preview': resolve(__dirname, 'screenshot-preview.html'),
        'pinned-screenshot': resolve(__dirname, 'pinned-screenshot.html'),
        'scroll-capture-bar': resolve(__dirname, 'scroll-capture-bar.html'),
        'video-editor': resolve(__dirname, 'video-editor.html'),
      },
    },
  },
});
