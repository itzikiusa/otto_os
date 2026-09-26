import { defineConfig, type Plugin } from 'vite'
import { readFileSync } from 'node:fs'
import { svelte } from '@sveltejs/vite-plugin-svelte'

// D2 0.1.33's bundled worker returns self.d2 immediately after go.run().
// WebKit can yield before Go registers that API, so the worker reports ready
// with an undefined compiler. Keep this compatibility patch version-scoped;
// re-evaluate/remove it when upgrading the upstream package.
function d2WorkerReady(): Plugin {
  return {
    name: 'd2-worker-ready',
    enforce: 'pre',
    transform(code, id) {
      if (!id.replaceAll('\\', '/').includes('/@terrastruct/d2/dist/browser/index.js')) return;
      const pkg = JSON.parse(readFileSync(new URL('./node_modules/@terrastruct/d2/package.json', import.meta.url), 'utf8'));
      const immediateReady = /go\.run\(result\.instance\);\s*return self\.d2;/;
      if (pkg.version !== '0.1.33' || !immediateReady.test(code)) {
        this.error('Re-evaluate the D2 WebKit readiness compatibility patch for this package version.');
      }
      const patched = code.replace(immediateReady, [
        'let startupError;',
        'void go.run(result.instance).catch(error => { startupError = error; });',
        'const started = Date.now();',
        'while (!self.d2) {',
        '  if (startupError) throw startupError;',
        '  if (Date.now() - started > 10000) throw new Error("D2 compiler startup timed out. Try reopening the diagram.");',
        '  await new Promise(resolve => setTimeout(resolve, 10));',
        '}',
        'return self.d2;',
      ].join('\n'));
      // Export the exact same worker/runtime assets for the isolated fallback.
      if (!code.includes('async function Af(') || !code.includes('new Blob([pf,If]')) {
        this.error('D2 packaged asset names changed; update the compatibility bridge.');
      }
      return patched + '\nexport const ottoD2Assets = async () => ({ source: pf + If, wasm: await Af("./d2.wasm") });\n';
    },
  };
}

// https://vite.dev/config/
export default defineConfig({
  plugins: [d2WorkerReady(), svelte()],
  optimizeDeps: { exclude: ['@terrastruct/d2'] },
  // The LSP client's nested open-rpc transport uses EventEmitter. Resolve its
  // Node-style import to the browser implementation in dev and production.
  resolve: { alias: [{ find: /^events$/, replacement: 'events/' }] },
})
