// data-integrity-5-02: Scheduled Tasks and Run with Otto are fed only by run
// events, so a reconnect / daemon restart must re-read them — but only a store
// someone already loaded, and only what that store shows (the list, the runs
// of expanded tasks, the open run and its timeline). Runs the REAL
// `events/routes/automation.ts` in a vm against stub stores.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { runInNewContext } from 'node:vm';
import ts from 'typescript';

function load(stores: Record<string, unknown>): { resyncAutomation: (ws: string | null) => void } {
  const out = ts.transpileModule(readFileSync(new URL('../src/lib/events/routes/automation.ts', import.meta.url), 'utf8'), {
    compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2022 },
  }).outputText;
  const exports: Record<string, any> = {};
  const require = (p: string): unknown => {
    if (p === '../../lazyModule') {
      return { lazyModule: (_load: unknown, key: string) => ({ peek: () => stores[key] ?? null, use() {} }) };
    }
    throw new Error(`unexpected import ${p}`);
  };
  runInNewContext(out, { exports, require, Object });
  return exports as { resyncAutomation: (ws: string | null) => void };
}

function stubs(log: string[]) {
  const scheduledTasks = {
    runsByTask: { t1: [], t2: [] } as Record<string, unknown[]>,
    loadList: async (ws: string) => void log.push(`tasks.list ${ws}`),
    loadRuns: async (id: string) => void log.push(`tasks.runs ${id}`),
  };
  const runWithOtto = {
    openId: 'r1' as string | null,
    loadList: async (ws: string) => void log.push(`runs.list ${ws}`),
    refreshRun: async (id: string) => void log.push(`runs.get ${id}`),
    loadEvents: async (id: string) => void log.push(`runs.events ${id}`),
  };
  return { scheduledTasks, runWithOtto };
}

test('a resync re-reads both automation stores: list, expanded runs, open run + timeline', () => {
  const log: string[] = [];
  const { resyncAutomation } = load(stubs(log));
  resyncAutomation('w1');
  assert.deepEqual(log, ['tasks.list w1', 'tasks.runs t1', 'tasks.runs t2', 'runs.list w1', 'runs.get r1', 'runs.events r1']);
});

test('a store nobody loaded is not imported or fetched', () => {
  const log: string[] = [];
  const { resyncAutomation } = load({});
  resyncAutomation('w1');
  assert.deepEqual(log, []);
});

test('the open run is read after the list load (another workspace closes it)', () => {
  const log: string[] = [];
  const s = stubs(log);
  s.runWithOtto.loadList = async (ws: string) => {
    log.push(`runs.list ${ws}`);
    s.runWithOtto.openId = null; // the store drops another workspace's open run synchronously
  };
  const { resyncAutomation } = load({ runWithOtto: s.runWithOtto });
  resyncAutomation('w2');
  assert.deepEqual(log, ['runs.list w2']);
});

test('no workspace: nothing to resync', () => {
  const log: string[] = [];
  const { resyncAutomation } = load(stubs(log));
  resyncAutomation(null);
  assert.deepEqual(log, []);
});
