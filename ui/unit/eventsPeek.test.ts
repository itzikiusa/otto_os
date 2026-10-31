// perf H1: page-store events in a document that never opened the page.
// The dispatcher used to `use()` (import) the API Client / Run with Otto /
// AWS / rooms / … stores for every such event, and the fresh store then
// fetched (API history ~every 150 ms during an automation run, one GET per
// `otto_run_updated`). These run the REAL dispatcher (events.svelte.ts,
// transpiled into a vm) against stub stores and count what it loads.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { existsSync, readFileSync } from 'node:fs';
import { runInNewContext } from 'node:vm';
import ts from 'typescript';

const SRC = new URL('../src/lib/', import.meta.url);

/** Transpile `file` (relative to src/lib) into a vm. Its relative imports
 *  reach `context.require` as src/lib-relative paths (`./stores/x.svelte`),
 *  and the feature route tables under `./events/` load for real, so the
 *  REAL dispatcher runs whichever file a route lives in. */
function cjs(file: string, context: Record<string, unknown>): Record<string, any> {
  const url = new URL(file, SRC);
  const out = ts.transpileModule(readFileSync(url, 'utf8'), {
    compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2022 },
  }).outputText;
  const exports: Record<string, any> = {};
  const outer = context.require as ((p: string) => unknown) | undefined;
  const require = (p: string): unknown => {
    if (!p.startsWith('.')) return outer?.(p);
    const lib = './' + new URL(p, url).pathname.slice(SRC.pathname.length);
    if (lib.startsWith('./events/')) {
      const rel = lib.slice(2);
      return cjs(existsSync(new URL(`${rel}.ts`, SRC)) ? `${rel}.ts` : `${rel}/index.ts`, context);
    }
    return outer?.(lib);
  };
  runInNewContext(out, { ...context, require, exports });
  return exports;
}

/** Callable, property-chaining stand-in for the shell modules the dispatcher
 *  imports statically (workspace, toasts, live, uiCommands, …). */
const inert: any = new Proxy(function () {}, {
  get: (_t, k) => (k === 'then' ? undefined : inert),
  apply: () => undefined,
});

/** Page store → [module path, export, key, methods the dispatcher calls]. */
const STORES = {
  apiClient: ['./stores/apiClient.svelte', ['noteHistoryAppended', 'noteRunProgress']],
  product: ['./stores/product.svelte', ['applyEvent', 'applyPlanRun']],
  loops: ['./stores/loops.svelte', ['applyEvent']],
  scheduledTasks: ['./stores/scheduledTasks.svelte', ['applyEvent']],
  browser: ['./stores/browser.svelte', ['applyEvent']],
  browserLive: ['./stores/browserLive.svelte', ['applyEvent']],
  personalAgents: ['./stores/personalAgents.svelte', ['applyRunEvent', 'applyRoomEvent']],
  k8s: ['./stores/k8s.svelte', ['applyEvent']],
  aws: ['./stores/aws.svelte', ['applyEvent']],
  database: ['./stores/database.svelte', ['applyAssistUpdate', 'setAssistSession']],
} as const;

/** One event per peeked handler (13 branches; multi-type branches send each). */
const PEEKED: Record<string, unknown>[] = [
  { type: 'api_history_appended', workspace_id: 'w1', entry_id: 'h1' },
  { type: 'api_run_progress', run_id: 'r1', status: 'running' },
  { type: 'product_changed', story_id: 's1', section: 'analysis', status: 'done' },
  { type: 'plan_run', story_id: 's1', session_ids: ['x'], interactive: false },
  { type: 'goal_loop_updated', loop_id: 'l1', status: 'running', phase: 'p', current_iteration: 1, progress_pct: 5 },
  { type: 'scheduled_task_run_updated', workspace_id: 'w1', task_id: 't1', run_id: 'tr1', status: 'ok' },
  { type: 'browser_tab_updated', workspace_id: 'w1', tab: { id: 'b1', url: 'https://x.test' } },
  { type: 'browser_annotation_added', workspace_id: 'w1', annotation: { id: 'a1', url: 'https://x.test' } },
  { type: 'browser_engine_install_updated', build: 'b', version: '1', state: 'downloading', received_bytes: 1, total_bytes: 2 },
  { type: 'personal_agent_run_updated', workspace_id: 'w1', agent_id: 'pa1', run_id: 'par1', status: 'ok' },
  { type: 'agent_room_message', workspace_id: 'w1', room_id: 'room1', message_id: 'm1', text: 'hi', created_at: 'now' },
  { type: 'k8s_cluster_updated', cluster_id: 'c1' },
  { type: 'k8s_install_updated', tool: 'kubectl' },
  { type: 'k8s_monitor_cycle', cluster_id: 'c1' },
  { type: 'aws_account_updated', account_id: 'acc1' },
  { type: 'aws_install_updated', tool: 'aws' },
];

function setup() {
  const loaded: string[] = [];
  const calls: string[] = [];
  const fetches: string[] = [];
  const fakes: Record<string, Record<string, (...a: unknown[]) => void>> = {};
  const byPath = new Map<string, Record<string, unknown>>();
  for (const [name, [path, methods]] of Object.entries(STORES)) {
    const fake: Record<string, (...a: unknown[]) => void> = {};
    for (const m of methods) fake[m] = (...a) => void calls.push(`${name}.${m}:${(a[0] as any)?.type ?? a[0]}`);
    fakes[name] = fake;
    byPath.set(path, { [name]: fake });
  }
  const api = new Proxy({}, { get: (_t, k) => (url: string) => fetches.push(`${String(k)} ${url}`) });
  const base = { setTimeout, clearTimeout, Promise, JSON, Object, Math, Date, Set, Map };
  const lazy = cjs('lazyModule.ts', base);
  let sock: any = null;
  const require = (p: string): unknown => {
    if (p === './lazyModule') return lazy;
    if (p === './api/client') return { api, getToken: () => 'tok', wsConnect: () => (sock = {}), resumeAltLoopback() {}, suspendAltLoopback() {} };
    if (p === './uiCommands') return { ...inert, handleUiFrame: () => false };
    const store = byPath.get(p);
    if (store) {
      loaded.push(p);
      return store;
    }
    return inert;
  };
  const $state = Object.assign((v: unknown) => v, { raw: (v: unknown) => v, snapshot: (v: unknown) => v });
  const { events } = cjs('events.svelte.ts', { ...base, require, $state, $derived: (v: unknown) => v });
  events.start();
  const send = async (ev: Record<string, unknown>) => {
    sock.onmessage({ data: JSON.stringify(ev) });
    for (let i = 0; i < 5; i++) await new Promise((r) => setTimeout(r, 0));
  };
  return { loaded, calls, fetches, fakes, send, announce: lazy.announceModule as (k: string, m: unknown) => void };
}

test('events: a document that never opened the page loads no page store and fetches nothing for its events', async () => {
  const h = setup();
  for (const ev of PEEKED) await h.send(ev);
  assert.deepEqual(h.loaded, [], 'no page store imported');
  assert.deepEqual(h.calls, [], 'no handler ran');
  assert.deepEqual(h.fetches, [], 'no request issued');
});

test('events: with the page open (its store announced), every peeked event reaches the store as before', async () => {
  const h = setup();
  for (const [name, fake] of Object.entries(h.fakes)) h.announce(name, fake);
  for (const ev of PEEKED) await h.send(ev);
  assert.deepEqual(h.loaded, [], 'peek never imports');
  assert.deepEqual(h.calls, [
    'apiClient.noteHistoryAppended:w1',
    'apiClient.noteRunProgress:r1',
    'product.applyEvent:product_changed',
    'product.applyPlanRun:plan_run',
    'loops.applyEvent:goal_loop_updated',
    'scheduledTasks.applyEvent:scheduled_task_run_updated',
    'browser.applyEvent:browser_tab_updated',
    'browser.applyEvent:browser_annotation_added',
    'browserLive.applyEvent:browser_engine_install_updated',
    'personalAgents.applyRunEvent:personal_agent_run_updated',
    'personalAgents.applyRoomEvent:agent_room_message',
    'k8s.applyEvent:k8s_cluster_updated',
    'k8s.applyEvent:k8s_install_updated',
    'k8s.applyEvent:k8s_monitor_cycle',
    'aws.applyEvent:aws_account_updated',
    'aws.applyEvent:aws_install_updated',
  ]);
});

test('events: an allow-listed payload event still loads its store, so the page sees it on mount', async () => {
  const h = setup();
  await h.send({ type: 'db_assist_updated', assist_id: 'a', connection_id: 'c', sql: 'select 1', note: null });
  assert.deepEqual(h.loaded, ['./stores/database.svelte']);
  assert.deepEqual(h.calls, ['database.applyAssistUpdate:a']);
});
