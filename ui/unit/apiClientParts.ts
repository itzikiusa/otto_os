// The API client store is a facade over parts in lib/stores/apiClient/
// (arch-6-03). Harnesses that run the store in a `node:vm` context load each
// part into the SAME sandbox (same `require` map, same rune shims), so the
// store's forwarded calls run the real part, not a stub.
import { readFileSync } from 'node:fs';
import { runInNewContext } from 'node:vm';
import ts from 'typescript';

const PARTS = { automations: '../src/lib/stores/apiClient/automations.svelte.ts' } as const;

export function apiClientParts() {
  const exports: Record<keyof typeof PARTS, Record<string, any>> = { automations: {} };
  return {
    /** strictRequire entries for the parts (matched by suffix). */
    entries: (Object.keys(PARTS) as (keyof typeof PARTS)[]).map((k) => [`/apiClient/${k}.svelte`, exports[k]] as const),
    /** Run every part in `context`'s sandbox; call before running the store. */
    load(context: Record<string, unknown>): void {
      for (const k of Object.keys(PARTS) as (keyof typeof PARTS)[]) {
        const src = readFileSync(new URL(PARTS[k], import.meta.url), 'utf8');
        const js = ts.transpileModule(src, { compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2022 } }).outputText;
        runInNewContext(js, { ...context, exports: exports[k] });
      }
    },
  };
}
