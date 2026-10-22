// Execute production stores with deterministic transports. Rune identities are
// intentional: these tests exercise asynchronous ownership, not DOM reactivity.
import { existsSync, readFileSync } from 'node:fs';
import { runInNewContext } from 'node:vm';
import ts from 'typescript';
import { webcrypto } from 'node:crypto';
import { performance } from 'node:perf_hooks';

export function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (reason: unknown) => void;
  const promise = new Promise<T>((yes, no) => { resolve = yes; reject = no; });
  return { promise, resolve, reject };
}

const SRC = new URL('../src/', import.meta.url);

/** Opt-in relative-import resolution for {@link loadSource}. */
export interface LoadOptions {
  /** Resolve a RELATIVE import the `imports` map doesn't name: first by its
   *  `src/`-relative key without extension (`'lib/stores/database/tabs.svelte'`
   *  — one stub key that works from any importing file), else by loading the
   *  real file (`.ts`, `.svelte.ts`, `/index.ts`) with the same map, globals
   *  and options. Each file loads once per top-level call, so a module two
   *  files import is one shared instance (a split store's parts see one
   *  another). Off by default: the map stays an explicit, exhaustive list. */
  relative?: boolean;
}

/** `spec` imported from `from`: the existing file it names, or null. */
function resolveRelative(spec: string, from: URL): URL | null {
  const base = new URL(spec, from);
  for (const suffix of ['', '.ts', '.svelte.ts', '/index.ts']) {
    const u = new URL(base.href + suffix);
    if (/\.ts$/.test(u.pathname) && existsSync(u)) return u;
  }
  return null;
}

/** `src/`-relative key of a resolved file, without its `.ts` extension. */
function srcKey(file: URL): string | null {
  return file.href.startsWith(SRC.href) ? file.href.slice(SRC.href.length).replace(/\.ts$/, '') : null;
}

export function loadSource(
  path: URL,
  imports: Record<string, unknown>,
  globals: Record<string, unknown> = {},
  options: LoadOptions = {},
  cache: Map<string, Record<string, any>> = new Map(),
) {
  const cached = cache.get(path.href);
  if (cached) return cached;
  const source = readFileSync(path, 'utf8');
  const compiled = ts.transpileModule(source, {
    compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2022 },
  }).outputText;
  const exports: Record<string, any> = {};
  // Registered before running, so an import cycle gets the partial exports
  // (CommonJS semantics) instead of recursing.
  if (options.relative) cache.set(path.href, exports);
  const storage = new Map<string, string>();
  const state = Object.assign((v: unknown) => v, { raw: (v: unknown) => v, snapshot: (v: unknown) => v });
  runInNewContext(compiled, {
    exports,
    require: (name: string) => {
      // Transport/router gained an opt-in dependency. Execute the real disabled
      // collector here so their existing behavior tests also cover its no-op path.
      if (!(name in imports) && (name === '../telemetry' || name === './telemetry')) {
        return loadSource(new URL('../src/lib/telemetry.ts', import.meta.url), {}, globals);
      }
      // The shared stale-response guard (lib/latest.ts) is pure TS: stores
      // that adopt it run the REAL helper unless a test overrides it.
      if (!(name in imports) && (name === '../latest' || name === './latest' || name === '../../lib/latest')) {
        return loadSource(new URL('../src/lib/latest.ts', import.meta.url), {}, globals);
      }
      // The "needs you" raise/lower rules (lib/attention.ts) are pure TS too.
      if (!(name in imports) && name === '../attention') {
        return loadSource(new URL('../src/lib/attention.ts', import.meta.url), {}, globals);
      }
      // Execute the real bounded worker helper; transport lanes are unused here.
      if (!(name in imports) && name === '../poll') {
        return loadSource(new URL('../src/lib/poll.ts', import.meta.url), {
          './api/lane': { inLane() { throw new Error('Unexpected lane call'); }, tagSignal() {} },
        }, globals);
      }
      // Pure, dependency-free shared helpers (load-error copy, the HTML
      // escaper, the byte formatter) run for real unless a test overrides them.
      const pure = /(?:^|\/)(loadError|html|metric-format)(?:\.ts)?$/.exec(name);
      if (!(name in imports) && pure) {
        return loadSource(new URL(`../src/lib/${pure[1]}.ts`, import.meta.url), {}, globals);
      }
      if (!(name in imports) && options.relative && name.startsWith('.')) {
        const file = resolveRelative(name, path);
        const key = file && srcKey(file);
        if (key && key in imports) return imports[key];
        if (file) return loadSource(file, imports, globals, options, cache);
      }
      if (!(name in imports)) throw new Error(`Missing fixture import: ${name}`);
      return imports[name];
    },
    $state: state,
    $derived: Object.assign((v: unknown) => v, { by: (fn: () => unknown) => fn() }),
    $effect: () => {},
    localStorage: {
      getItem: (key: string) => storage.get(key) ?? null,
      setItem: (key: string, value: string) => storage.set(key, value),
      removeItem: (key: string) => storage.delete(key),
    },
    URL, URLSearchParams, Response, Request, Headers, AbortController, DOMException,
    console, setTimeout, clearTimeout, setInterval, clearInterval, queueMicrotask,
    performance, crypto: webcrypto,
    ...globals,
  }, { timeout: 5000, filename: path.pathname });
  return exports;
}
