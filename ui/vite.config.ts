import { defineConfig } from 'vite';
import { svelte } from '@sveltejs/vite-plugin-svelte';

export default defineConfig({
  plugins: [svelte()],
  base: './',
  clearScreen: false,
  build: {
    // Un solo bundle local (Tauri / rust-embed): CodeMirror y sus lenguajes superan los 500 kB de aviso.
    chunkSizeWarningLimit: 1024,
  },
});
