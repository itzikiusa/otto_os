import { defineConfig } from 'vite'
import { svelte } from '@sveltejs/vite-plugin-svelte'

// https://vite.dev/config/
export default defineConfig({
  plugins: [svelte()],
  // The LSP client's nested open-rpc transport uses EventEmitter. Resolve its
  // Node-style import to the browser implementation in dev and production.
  resolve: { alias: [{ find: /^events$/, replacement: 'events/' }] },
})
