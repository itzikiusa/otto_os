import { defineConfig, mergeConfig } from 'vite';
import base from '../../vite.config';

// Real guest routes are same-origin. Proxy only to the explicitly selected
// throwaway daemon; never fall back to the user's installed daemon.
const port = process.env.OTTO_E2E_PORT;
if (!port || port === '7700') throw new Error('Set an isolated OTTO_E2E_PORT for the live room test');
export default mergeConfig(base, defineConfig({
  server: {proxy: {
    '/api/': {target: `http://127.0.0.1:${port}`, changeOrigin: true},
    '/ws/': {target: `http://127.0.0.1:${port}`, ws: true, changeOrigin: true},
  }},
}));
