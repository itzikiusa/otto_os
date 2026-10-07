#!/usr/bin/env node
// Run with `node ui/scripts/vite-lazy-deps-check.mjs` (install Chromium first).
// Exercise the real Vite/Svelte plugins in a fresh fixture/cache. A dependency
// hidden behind a lazy widget must be optimized before the first page mounts,
// so loading it cannot replace shared Svelte runtime chunks in that document.
import assert from 'node:assert/strict';
import { mkdtempSync, mkdirSync, writeFileSync, symlinkSync, readFileSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { createServer, loadConfigFromFile } from 'vite';
import { chromium, expect } from '@playwright/test';

const uiRoot = fileURLToPath(new URL('../', import.meta.url));
const configFile = join(uiRoot, 'vite.config.ts');

async function probe(legacyEntries) {
  const root = mkdtempSync(join(tmpdir(), 'otto-vite-lazy-deps-'));
  let server;
  let browser;
  try {
    mkdirSync(join(root, 'src'));
    symlinkSync(join(uiRoot, 'node_modules'), join(root, 'node_modules'), 'junction');
    writeFileSync(join(root, 'package.json'), '{"type":"module"}');
    writeFileSync(join(root, 'index.html'), '<main id="app"></main><script type="module" src="/src/main.ts"></script>');
    writeFileSync(join(root, 'src/main.ts'), `
      import { mount } from 'svelte';
      import App from './App.svelte';
      mount(App, { target: document.getElementById('app')! });
    `);
    // Keep the import in plain JS: Svelte's dev transform moves vite-ignore
    // comments out of import(), which would add an irrelevant scanner warning.
    writeFileSync(join(root, 'src/load.ts'), `
      export async function loadWidget() {
        const path = './Late.svelte';
        return (await import(/* @vite-ignore */ path)).default;
      }
    `);
    writeFileSync(join(root, 'src/App.svelte'), `
      <script>
        import { loadWidget } from './load';
        let Late = $state(null);
      </script>
      <button onclick={async () => { Late = await loadWidget(); }}>Load widget</button>
      {#if Late}<Late />{/if}
    `);
    writeFileSync(join(root, 'src/Late.svelte'), `
      <script>import QRCode from 'qrcode'; const ready = typeof QRCode.create === 'function';</script>
      <p>Widget ready: {ready}</p>
    `);
    const loaded = await loadConfigFromFile({ command: 'serve', mode: 'development' }, configFile);
    assert.ok(loaded, 'Vite configuration should load');
    const config = loaded.config;
    server = await createServer({
      ...config,
      configFile: false,
      root,
      cacheDir: join(root, 'cache'),
      optimizeDeps: {
        ...config.optimizeDeps,
        ...(legacyEntries ? { entries: ['index.html'] } : {}),
      },
      server: { host: '127.0.0.1', port: 0, strictPort: true },
      logLevel: 'warn',
    });
    await server.listen();
    const address = server.httpServer.address();
    assert.ok(address && typeof address !== 'string', 'Vite should bind an ephemeral TCP port');
    browser = await chromium.launch();
    const page = await browser.newPage({ serviceWorkers: 'block' });
    const errors = [];
    const runtimeUrls = new Set();
    page.on('pageerror', (error) => errors.push(error.message));
    page.on('request', (request) => {
      const url = new URL(request.url());
      if (/\/runtime-[^/]+\.js$/.test(url.pathname)) runtimeUrls.add(url.href);
    });
    await page.goto(`http://127.0.0.1:${address.port}`);
    await expect(page.getByRole('button', { name: 'Load widget' })).toBeVisible();
    const metadata = () => JSON.parse(readFileSync(join(root, 'cache/deps/_metadata.json'), 'utf8'));
    const before = metadata();
    await page.getByRole('button', { name: 'Load widget' }).click();
    await expect(page.getByText('Widget ready: true')).toBeVisible();
    const after = metadata();
    assert.deepEqual(errors, [], 'The lazy Svelte widget should render without runtime errors');
    if (legacyEntries) {
      assert.equal(Boolean(before.optimized.qrcode), false, 'Control fixture must hide its late dependency from the old scan');
      assert.notEqual(after.browserHash, before.browserHash, 'Old entry-only scanning must reproduce dependency hash churn');
    } else {
      assert.ok(before.optimized.qrcode, 'Lazy widget dependency must be optimized before the first mount');
      assert.equal(after.browserHash, before.browserHash, 'Opening a lazy widget must preserve the dependency hash');
      assert.equal(runtimeUrls.size, 1, 'The document must load exactly one Svelte runtime');
    }
    console.log(`[vite-lazy-deps] ${legacyEntries ? 'legacy control reproduces hash churn' : 'current configuration keeps one stable runtime'}`);
  } finally {
    try {
      await browser?.close();
    } finally {
      try {
        await server?.close();
      } finally {
        rmSync(root, { recursive: true, force: true });
      }
    }
  }
}

await probe(true);
await probe(false);
