import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { runInNewContext } from 'node:vm';
import ts from 'typescript';

// Execute the actual store methods; the harness supplies only transport and
// browser globals. State proxies are immaterial to these transition tests.
class ApiError extends Error { status = 409; }
const note = (path: string, raw = `disk-${path}`) => ({raw, meta: {path, hash: raw}, outgoing: []});
function setup(overrides: Record<string, unknown> = {}) {
  const api = {
    writeVaultNote: async (_ws: string, _id: number, body: {path: string; content: string}) => note(body.path, body.content).meta,
    vaultNote: async (_ws: string, _id: number, p: string) => note(p),
    vaultStatus: async () => ({scan_state: 'idle', last_scan_at: 'new', notes: 1, links: 0, unresolved: 0}),
    vaultDir: async () => ({entries: []}), vaultTags: async () => [], vaultBacklinks: async () => [],
    ...overrides,
  };
  const source = readFileSync(new URL('../src/modules/vault/vault.svelte.ts', import.meta.url), 'utf8');
  const context = {
    exports: {} as Record<string, any>, $state: (v: unknown) => v,
    require: (p: string) => p.endsWith('/vault') ? api : p.endsWith('/client') ? {ApiError}
      : p.includes('workspace.svelte') ? {ws: {current: {id: 'ws'}}}
      : p.includes('toast') ? {toasts: {error() {}, success() {}}} : {},
    localStorage: {setItem() {}, getItem() {return null;}}, setTimeout, clearTimeout, setInterval, clearInterval, URL,
  };
  runInNewContext(ts.transpileModule(source, {compilerOptions: {module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2022}}).outputText, context);
  const v = context.exports.vault;
  v.current = {id: 1}; v.note = note('a.md'); v.notePath = 'a.md'; v.draft = 'local edit'; v.dirty = true;
  v.tabs = [{kind: 'note', path: 'a.md'}]; v.activeTab = 0;
  return v;
}

test('failed save retains draft, conflict and current tab on navigation', async () => {
  const v = setup({writeVaultNote: async () => {throw new ApiError('changed');}});
  await v.open('b.md');
  assert.equal(v.notePath, 'a.md'); assert.equal(v.draft, 'local edit');
  assert.equal(v.conflict, true); assert.equal(v.tabs[0].path, 'a.md');
});

test('last dirty tab stays open when saving fails', async () => {
  const v = setup({writeVaultNote: async () => {throw new Error('offline');}});
  await v.closeTab(0);
  assert.equal(v.notePath, 'a.md'); assert.equal(v.tabs.length, 1); assert.equal(v.dirty, true);
});

test('scan completion refreshes a clean open note even when counts are unchanged', async () => {
  const v = setup(); v.dirty = false;
  v.status = {scan_state: 'idle', last_scan_at: 'old', notes: 1, links: 0, unresolved: 0};
  v.note = note('a.md', 'old content');
  await v.refreshStatus();
  assert.equal(v.note.raw, 'disk-a.md');
});

test('scan completion preserves local draft and flags the external change', async () => {
  const v = setup(); v.status = {scan_state: 'idle', last_scan_at: 'old', notes: 1, links: 0, unresolved: 0};
  v.note = note('a.md', 'old content');
  await v.refreshStatus();
  assert.equal(v.draft, 'local edit'); assert.equal(v.conflict, true);
});

test('navigation awaits the in-flight save and drains edits typed during it', async () => {
  let release!: () => void;
  const gate = new Promise<void>(resolve => {release = resolve;});
  const writes: string[] = [];
  const v = setup({writeVaultNote: async (_ws: string, _id: number, body: {path: string; content: string}) => {
    writes.push(body.content);
    if (writes.length === 1) await gate;
    return note(body.path, body.content).meta;
  }});
  const saving = v.saveNow();
  v.draft = 'latest local edit'; v.dirty = true;
  const navigating = v.open('b.md');
  assert.equal(v.notePath, 'a.md');
  release(); await Promise.all([saving, navigating]);
  assert.deepEqual(writes, ['local edit', 'latest local edit']);
  assert.equal(v.notePath, 'b.md');
});

test('vault switching retains a dirty note if the save fails', async () => {
  const v = setup({writeVaultNote: async () => {throw new Error('offline');}});
  v.vaults = [{id: 1}, {id: 2}];
  await v.select(2);
  assert.equal(v.current.id, 1); assert.equal(v.draft, 'local edit');
});
