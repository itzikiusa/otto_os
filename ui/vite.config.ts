import { defineConfig, type Plugin } from 'vite'
import { cpSync, createReadStream, existsSync, readFileSync, readdirSync, statSync } from 'node:fs'
import { join, normalize, resolve, sep } from 'node:path'
import { fileURLToPath } from 'node:url'
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

// Help's guide index (modules/help/sectionsIndex.ts): every guide's frontmatter
// (title, group, route, summary) and its "Keyboard shortcuts" table, without
// the guide text — the rail and ⌘K need only that; the full text is one lazy
// chunk (modules/help/sections.ts) loaded when Help opens.
const HELP_SECTIONS = fileURLToPath(new URL('./src/modules/help/sections', import.meta.url))
function helpGuideIndex(): Plugin {
  const ID = 'virtual:help-guide-index'
  return {
    name: 'help-guide-index',
    resolveId(id) {
      return id === ID ? '\0' + ID : undefined
    },
    load(id) {
      if (id !== '\0' + ID) return
      this.addWatchFile(HELP_SECTIONS)
      const out: Record<string, string> = {}
      for (const name of readdirSync(HELP_SECTIONS).filter((f) => f.endsWith('.md')).sort()) {
        const path = join(HELP_SECTIONS, name)
        this.addWatchFile(path)
        const raw = readFileSync(path, 'utf8').replace(/\r\n?/g, '\n')
        const fm = /^\uFEFF?---\n[\s\S]*?\n---\n/.exec(raw)?.[0] ?? ''
        const keys = /^## Keyboard shortcuts\n[\s\S]*?(?=^## |(?![\s\S]))/m.exec(raw)?.[0] ?? ''
        out['./sections/' + name] = fm + keys
      }
      return 'export default ' + JSON.stringify(out)
    },
  }
}

// Excalidraw's hand-drawn fonts, served by Otto itself instead of unpkg.com
// (the desktop CSP is `font-src 'self' data:`, so the CDN was blocked there
// anyway, and the first canvas open no longer waits on the internet). The
// canvases set `EXCALIDRAW_ASSET_PATH` to `<base>assets/excalidraw/`; the
// package resolves `fonts/<Family>/<file>.woff2` under it. Dev serves them
// straight from node_modules; a build copies them into dist. Xiaolai (the
// 12 MB CJK handwriting face) is skipped to keep the embedded UI small — CJK
// text falls back to the system font; a missing file is a clean 404 under
// `assets/` (never index.html).
const EXCALIDRAW_FONTS = fileURLToPath(new URL('./node_modules/@excalidraw/excalidraw/dist/prod/fonts', import.meta.url)).replace(/[\\/]$/, '')
const EXCALIDRAW_FONTS_URL = '/assets/excalidraw/fonts/'
const EXCALIDRAW_SKIP = new Set(['Xiaolai'])
function excalidrawFonts(): Plugin {
  let outDir = 'dist'
  return {
    name: 'excalidraw-local-fonts',
    configResolved(c) {
      outDir = resolve(c.root, c.build.outDir)
    },
    configureServer(server) {
      server.middlewares.use((req, res, next) => {
        const url = (req.url ?? '').split('?')[0]
        if (!url.startsWith(EXCALIDRAW_FONTS_URL)) return next()
        const rel = normalize(decodeURIComponent(url.slice(EXCALIDRAW_FONTS_URL.length)))
        const file = join(EXCALIDRAW_FONTS, rel)
        const family = rel.split(sep)[0]
        if (!file.startsWith(EXCALIDRAW_FONTS + sep) || EXCALIDRAW_SKIP.has(family) || !existsSync(file) || !statSync(file).isFile()) {
          res.statusCode = 404
          return res.end()
        }
        res.setHeader('Content-Type', 'font/woff2')
        res.setHeader('Cache-Control', 'public, max-age=31536000, immutable')
        createReadStream(file).pipe(res)
      })
    },
    writeBundle() {
      if (!existsSync(EXCALIDRAW_FONTS)) {
        this.error('@excalidraw/excalidraw fonts not found — the local-font copy needs re-evaluating for this package version.')
      }
      cpSync(EXCALIDRAW_FONTS, join(outDir, 'assets/excalidraw/fonts'), {
        recursive: true,
        filter: (src) => !EXCALIDRAW_SKIP.has(src.slice(EXCALIDRAW_FONTS.length + 1).split(sep)[0]),
      })
    },
  }
}

// https://vite.dev/config/
export default defineConfig({
  plugins: [d2WorkerReady(), excalidrawFonts(), helpGuideIndex(), svelte()],
  // dist/.vite/manifest.json feeds scripts/bundle-budget.mjs (the CI byte
  // budget walks each entry's static-import closure from it).
  build: { manifest: true },
  optimizeDeps: {
    // Scan lazy pages/widgets before serving the first page. Discovering their
    // dependencies later changes shared chunk hashes and can load two Svelte
    // runtimes in one document (new widgets then crash in get_first_child).
    entries: ['index.html', 'src/**/*.{ts,svelte}'],
    exclude: ['@terrastruct/d2'],
  },
  // The LSP client's nested open-rpc transport uses EventEmitter. Resolve its
  // Node-style import to the browser implementation in dev and production.
  resolve: { alias: [{ find: /^events$/, replacement: 'events/' }] },
})
