// The API client store is a facade over parts in lib/stores/apiClient/
// (arch-6-03). Harnesses that run the store in a `node:vm` context load each
// part into the SAME sandbox (same `require` map, same rune shims), so the
// store's forwarded calls run the real part, not a stub.
import { readFileSync } from 'node:fs';
import { runInNewContext } from 'node:vm';
import ts from 'typescript';

/** Import specifier (without extension) → source file. `model` first: the
 *  other parts read its helpers. */
const PARTS = {
  model: '../src/lib/stores/apiClient/model.ts',
  curl: '../src/lib/stores/apiClient/curl.ts',
  'tabs.svelte': '../src/lib/stores/apiClient/tabs.svelte.ts',
  'cookies.svelte': '../src/lib/stores/apiClient/cookies.svelte.ts',
  'execute.svelte': '../src/lib/stores/apiClient/execute.svelte.ts',
  'drafts.svelte': '../src/lib/stores/apiClient/drafts.svelte.ts',
  collectionsSync: '../src/lib/stores/apiClient/collectionsSync.ts',
  'automations.svelte': '../src/lib/stores/apiClient/automations.svelte.ts',
} as const;
type Part = keyof typeof PARTS;

export function apiClientParts() {
  const names = Object.keys(PARTS) as Part[];
  const exports = Object.fromEntries(names.map((k) => [k, {}])) as Record<Part, Record<string, any>>;
  return {
    /** strictRequire entries for the parts: the store imports them as
     *  `./apiClient/<k>`, the parts import each other as `./<k>`. */
    entries: names.flatMap((k) => [[`/apiClient/${k}`, exports[k]], [`./${k}`, exports[k]]] as const),
    /** Run every part in `context`'s sandbox; call before running the store. */
    load(context: Record<string, unknown>): void {
      for (const k of names) {
        const src = readFileSync(new URL(PARTS[k], import.meta.url), 'utf8');
        const js = ts.transpileModule(src, { compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2022 } }).outputText;
        runInNewContext(js, { ...context, exports: exports[k] });
      }
    },
  };
}
