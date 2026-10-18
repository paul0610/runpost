import { defineConfig } from 'vitest/config';
import { svelte } from '@sveltejs/vite-plugin-svelte';

export default defineConfig({
  plugins: [svelte()],
  // Runtime de cliente de Svelte (los módulos .svelte.ts usan runas).
  resolve: { conditions: ['browser'] },
  test: {
    environment: 'jsdom',
    include: ['src/**/*.test.ts'],
    // Sin esto Vitest entrega los .css vacíos y contrast.test.ts no vería los tokens.
    css: { include: [/tokens\.css/] },
  },
});
