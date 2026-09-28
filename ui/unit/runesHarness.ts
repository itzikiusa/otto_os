// Run a production `.svelte.ts` rune module under the REAL Svelte client
// runtime (not the identity shims in sourceHarness.ts): strip the types,
// compile with svelte/compiler's compileModule, point the `svelte` imports at
// the client build, and import the result. For reactivity bugs that only the
// real scheduler shows (teardown `old_values`, derived reconnects, batches).
import { mkdtempSync, readFileSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { pathToFileURL, fileURLToPath } from 'node:url';
import ts from 'typescript';
import { compileModule } from 'svelte/compiler';

const SVELTE = new URL('../node_modules/svelte/src/', import.meta.url);
const CLIENT: Record<string, string> = {
  svelte: new URL('index-client.js', SVELTE).href,
  'svelte/internal/client': new URL('internal/client/index.js', SVELTE).href,
  'svelte/internal/disclose-version': new URL('internal/disclose-version.js', SVELTE).href,
  'svelte/internal/flags/async': new URL('internal/flags/async.js', SVELTE).href,
  'svelte/internal/flags/legacy': new URL('internal/flags/legacy.js', SVELTE).href,
};

let dir: string | null = null;

/** Compile + import a rune module. Its own imports must be `svelte` only
 *  (or absolute), which is what keeps the harness honest about scope. */
export async function importRunes<T = Record<string, unknown>>(path: URL): Promise<T> {
  const source = readFileSync(path, 'utf8');
  const js = ts.transpileModule(source, {
    compilerOptions: { module: ts.ModuleKind.ESNext, target: ts.ScriptTarget.ES2022, verbatimModuleSyntax: false },
  }).outputText;
  const filename = fileURLToPath(path).replace(/\.ts$/, '.js');
  const out = compileModule(js, { filename, generate: 'client', dev: false }).js.code.replace(
    /from\s+['"](svelte(?:\/[^'"]*)?)['"]/g,
    (m, spec: string) => {
      const target = CLIENT[spec];
      if (!target) throw new Error(`runesHarness: no client mapping for '${spec}'`);
      return `from '${target}'`;
    },
  );
  dir ??= mkdtempSync(join(tmpdir(), 'otto-runes-'));
  const file = join(dir, `${Math.random().toString(36).slice(2)}.mjs`);
  writeFileSync(file, out);
  return (await import(pathToFileURL(file).href)) as T;
}

/** The real client runtime entry points a test drives effects with. */
export async function svelteClient(): Promise<{
  flushSync: (fn?: () => void) => void;
  effectRoot: (fn: () => void | (() => void)) => () => void;
  effect: (fn: () => void | (() => void)) => void;
  state: <V>(v: V) => { v: V };
  get: <V>(s: { v: V }) => V;
  set: <V>(s: { v: V }, v: V) => void;
}> {
  const svelte = (await import(CLIENT.svelte)) as { flushSync: (fn?: () => void) => void };
  const $ = (await import(CLIENT['svelte/internal/client'])) as Record<string, (...a: unknown[]) => unknown>;
  return {
    flushSync: svelte.flushSync,
    effectRoot: (fn) => $.effect_root(fn) as () => void,
    effect: (fn) => void $.user_effect(fn),
    state: (v) => $.state(v) as { v: typeof v },
    get: (s) => $.get(s) as never,
    set: (s, v) => void $.set(s, v),
  };
}
