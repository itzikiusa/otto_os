// Execute production stores with deterministic transports. Rune identities are
// intentional: these tests exercise asynchronous ownership, not DOM reactivity.
import { readFileSync } from 'node:fs';
import { runInNewContext } from 'node:vm';
import ts from 'typescript';

export function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (reason: unknown) => void;
  const promise = new Promise<T>((yes, no) => { resolve = yes; reject = no; });
  return { promise, resolve, reject };
}

export function loadSource(path: URL, imports: Record<string, unknown>, globals: Record<string, unknown> = {}) {
  const source = readFileSync(path, 'utf8');
  const compiled = ts.transpileModule(source, {
    compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2022 },
  }).outputText;
  const exports: Record<string, any> = {};
  const storage = new Map<string, string>();
  const state = Object.assign((v: unknown) => v, { raw: (v: unknown) => v, snapshot: (v: unknown) => v });
  runInNewContext(compiled, {
    exports,
    require: (name: string) => {
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
    ...globals,
  }, { timeout: 5000, filename: path.pathname });
  return exports;
}
