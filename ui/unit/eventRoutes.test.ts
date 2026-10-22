// architecture-2-04: WebSocket events route through a typed table split per
// feature (lib/events/routes/*.ts) instead of a hand-written if/else chain. An
// event type no table lists used to be silently dropped: this pins that every
// `OttoEvent` type is routed, or deliberately listed as reaching its consumers
// through `liveEvents` subscribers only.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync, readdirSync, existsSync } from 'node:fs';
import { runInNewContext } from 'node:vm';
import ts from 'typescript';

const LIB = new URL('../src/lib/', import.meta.url);

/** Callable, property-chaining stand-in for every store / shell module. */
const inert: any = new Proxy(function () {}, {
  get: (_t, k) => (k === 'then' ? undefined : inert),
  apply: () => undefined,
});

/** Transpile `file` (relative to src/lib) into a vm; relative imports under
 *  `events/` load for real (recursively), everything else is `inert`. */
export function loadEventsModule(file: string, ctx: Record<string, unknown> = {}, stub: (lib: string) => unknown = () => inert): Record<string, any> {
  const url = new URL(file, LIB);
  const out = ts.transpileModule(readFileSync(url, 'utf8'), {
    compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2022 },
  }).outputText;
  const exports: Record<string, any> = {};
  const require = (p: string): unknown => {
    if (!p.startsWith('.')) return inert;
    const abs = new URL(p, url);
    const lib = './' + abs.pathname.slice(LIB.pathname.length);
    if (lib.startsWith('./events/')) {
      const rel = lib.slice(2);
      const path = existsSync(new URL(`${rel}.ts`, LIB)) ? `${rel}.ts` : `${rel}/index.ts`;
      return loadEventsModule(path, ctx, stub);
    }
    return stub(lib);
  };
  const $state = Object.assign((v: unknown) => v, { raw: (v: unknown) => v, snapshot: (v: unknown) => v });
  runInNewContext(out, {
    exports, require, $state, $derived: (v: unknown) => v,
    setTimeout, clearTimeout, Promise, JSON, Object, Math, Date, Set, Map, console,
    ...ctx,
  });
  return exports;
}

/** Every `type: '…'` member of the `OttoEvent` union (types.ts or the split
 *  types/ directory), following named members (`| WorkbenchDocChangedEvent`). */
function ottoEventTypes(): string[] {
  const files = [new URL('api/types.ts', LIB)];
  const dir = new URL('api/types/', LIB);
  if (existsSync(dir)) for (const f of readdirSync(dir)) if (f.endsWith('.ts')) files.push(new URL(f, dir));
  const src = files.map((f) => readFileSync(f, 'utf8')).join('\n');
  const start = src.indexOf('export type OttoEvent =');
  assert.ok(start >= 0, 'OttoEvent union not found');
  // The union's members are object literals and named types; it ends at the
  // first line that closes with `;`.
  const lines = src.slice(start).split('\n');
  const body: string[] = [];
  for (const line of lines) {
    body.push(line);
    if (/^\s*\|[^{]*;\s*$/.test(line) || /^\s*};\s*$/.test(line) || /^\s*\|\s*\{.*\};\s*$/.test(line)) break;
  }
  const union = body.join('\n');
  const types = [...union.matchAll(/type:\s*'(\w+)'/g)].map((m) => m[1]);
  for (const m of union.matchAll(/^\s*\|\s*([A-Z]\w+)\s*;?\s*$/gm)) {
    const decl = new RegExp(`(?:interface|type)\\s+${m[1]}\\b[^{]*\\{([\\s\\S]*?)\\n\\}`).exec(src);
    assert.ok(decl, `named union member ${m[1]} not found`);
    const t = /type:\s*'(\w+)'/.exec(decl[1]);
    assert.ok(t, `${m[1]} has no literal type`);
    types.push(t[1]);
  }
  return [...new Set(types)];
}

function load() {
  const routes = loadEventsModule('events/routes/index.ts');
  const events = loadEventsModule('events.svelte.ts', {}, (lib) =>
    lib === './lazyModule' ? { lazyModule: () => inert } : inert,
  );
  return { routes, events };
}

test('every OttoEvent type is routed or listed as liveEvents-only', () => {
  const { routes, events } = load();
  const routed = new Set(Object.keys(events.eventRoutes));
  const liveOnly = new Set<string>(routes.LIVE_ONLY_EVENTS);
  const types = ottoEventTypes();
  assert.ok(types.length > 60, `parsed ${types.length} union members`);
  const missing = types.filter((t) => !routed.has(t) && !liveOnly.has(t));
  assert.deepEqual(missing, [], 'route these types (events/routes/*.ts) or list them in LIVE_ONLY_EVENTS');
  const both = [...liveOnly].filter((t) => routed.has(t));
  assert.deepEqual(both, [], 'a liveEvents-only type must not also have a route');
  const stale = [...routed, ...liveOnly].filter((t) => !types.includes(t));
  assert.deepEqual(stale, [], 'routes / listings for types the union no longer has');
});

test('the feature tables do not route one type twice', () => {
  const dir = new URL('events/routes/', LIB);
  const seen = new Map<string, string>();
  for (const f of readdirSync(dir)) {
    if (!f.endsWith('.ts') || f === 'index.ts' || f === 'types.ts' || f === 'stores.ts') continue;
    const table = Object.entries(loadEventsModule(`events/routes/${f}`)).find(([k]) => k.endsWith('Routes'));
    assert.ok(table, `${f} exports no …Routes table`);
    for (const k of Object.keys(table[1])) {
      assert.ok(!seen.has(k), `${k} routed by both ${seen.get(k)} and ${f}`);
      seen.set(k, f);
    }
  }
});
