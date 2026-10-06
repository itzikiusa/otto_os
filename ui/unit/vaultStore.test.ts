import { refreshVisibleTree } from '../src/modules/vault/treeRefresh.ts';
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { runInNewContext } from 'node:vm';
import ts from 'typescript';
import { deferred } from './sourceHarness.ts';
import { strictRequire, unused } from './strictRequire.ts';

// Execute the actual store methods; the harness supplies only transport and
// browser globals. State proxies are immaterial to these transition tests.
class ApiError extends Error { status = 409; }
/** Toast warnings and localStorage writes the store made (reset per setup). */
const warned: string[] = [];
const stored: Array<[string, string]> = [];
const note = (path: string, raw = `disk-${path}`) => ({raw, meta: {path, hash: raw}, outgoing: []});
function setup(overrides: Record<string, unknown> = {}) {
  const api = {
    writeVaultNote: async (_ws: string, _id: number, body: {path: string; content: string}) => note(body.path, body.content).meta,
    vaultNote: async (_ws: string, _id: number, p: string) => note(p),
    vaultStatus: async () => ({scan_state: 'idle', last_scan_at: 'new', notes: 1, links: 0, unresolved: 0}),
    vaultDir: async () => ({entries: []}), vaultTags: async () => [], vaultBacklinks: async () => [],
    ...overrides,
  };
  warned.length = 0; stored.length = 0;
  const source = readFileSync(new URL('../src/modules/vault/vault.svelte.ts', import.meta.url), 'utf8');
  const context = {
    exports: {} as Record<string, any>, $state: (v: unknown) => v,
    require: strictRequire([
      ['/treeRefresh', {refreshVisibleTree}],
      ['/api/vault', api],
      ['/api/client', {ApiError, authedBlobUrl: unused('client.authedBlobUrl')}],
      ['/api/types', unused('/api/types')],
      ['/workspace.svelte', {ws: {current: {id: 'ws'}}}],
      ['/toast.svelte', {toasts: {error() {}, success() {}, warn(t: string) { warned.push(t); }}}],
      ['/storage', {lsGet: () => null, lsSet(k: string, v: string) { stored.push([k, v]); }, lsRemove() {}}],
      ['/loadError', unused('/loadError')],
      ['/confirm.svelte', unused('/confirm.svelte')],
      ['/poll', unused('/poll')],
    ]),
    localStorage: {setItem() {}, getItem() {return null;}}, setTimeout, clearTimeout, setInterval, clearInterval, URL,
  };
  runInNewContext(ts.transpileModule(source, {compilerOptions: {module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2022}}).outputText, context);
  const v = context.exports.vault;
  v.current = {id: 1}; v.note = note('a.md'); v.notePath = 'a.md'; v.draft = 'local edit'; v.dirty = true;
  v.tabs = [{kind: 'note', path: 'a.md'}]; v.activeTab = 0;
  return v;
}

test('tag refresh failure keeps the last good tags available', async () => {
  const v = setup({ vaultTags: async () => { throw new Error('Index unavailable'); } });
  v.tags = [{ tag: 'release', count: 4 }];
  await v.loadTags();
  assert.equal(v.tags.length, 1);
  assert.equal(v.tags[0].tag, 'release');
});

test('switcher transport failure remains distinguishable from an empty lookup', async () => {
  const v = setup({ vaultSwitcher: async () => { throw new Error('Index unavailable'); } });
  await assert.rejects(v.switcherQuery('Existing note'), /Index unavailable/);
});

test('backlink refresh failure retains current note links and exposes retry state', async () => {
  let fail = true;
  const v = setup({vaultBacklinks: async () => {
    if (fail) throw new Error('Index unavailable');
    return [];
  }});
  v.backlinks = [{path: 'source.md', kind: 'wiki', context: 'known source'}];
  await v.reloadBacklinks();
  assert.equal(v.backlinks.length, 1);
  assert.match(v.backlinksError, /Index unavailable/);
  assert.equal(v.backlinksLoading, false);
  fail = false;
  await v.reloadBacklinks();
  assert.equal(v.backlinks.length, 0, 'successful empty response is distinct from failure');
  assert.equal(v.backlinksError, '');
});

test('backlink pending and stale failure stay with the current note request', async () => {
  const old = deferred<unknown>();
  const current = deferred<unknown>();
  let calls = 0;
  const v = setup({vaultBacklinks: () => ++calls === 1 ? old.promise : current.promise});
  const first = v.reloadBacklinks();
  assert.equal(v.backlinksLoading, true);
  v.notePath = 'b.md';
  const second = v.reloadBacklinks();
  old.reject(new Error('Old A error'));
  await first;
  assert.equal(v.backlinksLoading, true, 'old finally cannot settle B');
  assert.equal(v.backlinksError, '');
  current.resolve([{path: 'b-source.md', kind: 'wiki', context: ''}]);
  await second;
  assert.equal(v.backlinksLoading, false);
  assert.equal(v.backlinks[0].path, 'b-source.md');
});

test('switching notes clears old backlinks before the new read and never restores old errors', async () => {
  const next = deferred<unknown>();
  const v = setup({vaultBacklinks: () => next.promise});
  v.dirty = false;
  v.backlinks = [{path: 'a-source.md', kind: 'wiki', context: ''}];
  v.backlinksError = 'A failed';
  await v.open('b.md');
  assert.equal(v.backlinks.length, 0);
  assert.equal(v.backlinksError, '');
  assert.equal(v.backlinksLoading, true);
  next.resolve([]);
  await new Promise(resolve => setTimeout(resolve, 0));
  assert.equal(v.backlinksLoading, false);
});

test('backlink refresh resets the visible window and allows rehydrating edited sources', async () => {
  let revision = 'first';
  const offsets: number[] = [];
  const v = setup({vaultStatus: async () => ({generation:revision,scan_state:'idle',last_scan_at:revision,notes:1,links:226,attachments:0}),
    vaultBacklinks: async (_ws: string, _id: number, _path: string, offset = 0) => {
    offsets.push(offset);
    return Array.from({length:226}, (_, i) => ({path:`source-${i}.md`, kind:'wiki', title:`Source ${i}`,
      context:i >= offset && i < offset + 100 ? `${revision}-${i}` : ''}));
  }});
  await v.reloadBacklinks();
  await v.loadBacklinkContexts(100); await v.loadBacklinkContexts(200);
  assert.equal(v.visibleBacklinks, 300);
  assert.equal(v.backlinks[225].context, 'first-225');
  v.dirty = false;
  v.status = {generation:revision,scan_state:'idle',last_scan_at:revision,notes:1,links:226,attachments:0};
  revision = 'edited';
  await v.refreshStatus();
  // The external-generation path intentionally schedules the backlink reload.
  await new Promise(resolve => setTimeout(resolve, 0));
  assert.equal(v.visibleBacklinks, 100, 'refresh must not leave unhydrated rows expanded');
  assert.equal(v.backlinks[0].context, 'edited-0');
  assert.equal(v.backlinks[125].context, '', 'do not retain snippets from obsolete source hashes');
  await v.loadBacklinkContexts(v.visibleBacklinks);
  assert.equal(v.visibleBacklinks, 200); assert.equal(v.backlinks[125].context, 'edited-125');
  assert.deepEqual(offsets, [0,100,200,0,100]);
});

test('failed save retains draft, conflict and current tab on navigation', async () => {
  const v = setup({writeVaultNote: async () => {throw new ApiError('conflict: note changed on disk (hash x)');}});
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

test('refresh only requests visible branches and invalidates collapsed children', async () => {
  const calls: string[] = [];
  const entry = {kind: 'dir', path: 'closed', name: 'closed', children: 1};
  const v = setup({vaultDir: async (_ws: string, _id: number, path: string) => {
    calls.push(path);
    return {entries: path === '' ? [entry] : []};
  }});
  v.roots = [{entry, depth: 0, open: false, loaded: true, loading: false, children: []}];
  await v.refreshTree();
  assert.deepEqual(calls, [''], 'collapsed subtree must not cause a directory request');
  assert.equal(v.roots[0].loaded, false, 'reopening requires fresh children');
});

test('collapse during a refresh is preserved when pending directory results arrive', async () => {
  let release!: () => void;
  let started!: () => void;
  const gate = new Promise<void>(r => {release = r;});
  const pending = new Promise<void>(r => {started = r;});
  const entry = {kind: 'dir', path: 'folder', name: 'folder', children: 1};
  const v = setup({vaultDir: async (_ws: string, _id: number, path: string) => {
    if (path === 'folder') {started(); await gate;}
    return {entries: path === '' ? [entry] : []};
  }});
  v.roots = [{entry, depth: 0, open: true, loaded: true, loading: false, children: []}];
  const refresh = v.refreshTree();
  await pending;
  v.collapseAll();
  release(); await refresh;
  assert.equal(v.roots[0].open, false, 'stale refresh must not reopen a collapsed branch');
});

test('reopening a still-loading branch ignores the older directory response', async () => {
  const releases: ((value: unknown) => void)[] = [];
  const entry = {kind: 'dir', path: 'folder', name: 'folder', children: 1};
  const v = setup({vaultDir: () => new Promise(resolve => releases.push(resolve))});
  v.roots = [{entry, depth: 0, open: false, loaded: false, loading: false, children: []}];
  const node = v.roots[0];
  const first = v.toggleDir(node);
  await v.toggleDir(node);
  const latest = v.toggleDir(node);
  releases[1]({entries: [{kind: 'note', path: 'folder/new.md', name: 'new.md'}]});
  await latest;
  releases[0]({entries: [{kind: 'note', path: 'folder/old.md', name: 'old.md'}]});
  await first;
  assert.equal(node.children[0].entry.path, 'folder/new.md');
});

test('transient busy 409 keeps the draft, raises no conflict banner and retries', async () => {
  let calls = 0;
  const v = setup({writeVaultNote: async (_ws: string, _id: number, body: {path: string; content: string}) => {
    calls += 1;
    if (calls === 1) throw new ApiError('conflict: Vault indexing is busy; retry the operation');
    return note(body.path, body.content).meta;
  }});
  assert.equal(await v.saveNow(), false);
  assert.equal(v.conflict, false); assert.equal(v.draft, 'local edit'); assert.equal(v.dirty, true);
  await new Promise((r) => setTimeout(r, 700));
  assert.equal(calls, 2); assert.equal(v.dirty, false); assert.equal(v.conflict, false);
});

test('an equal status poll keeps the status object (graph effects key off it)', async () => {
  const s = {id: 1, generation: '7', scan_state: 'idle', last_scan_at: 'x', notes: 1, links: 0, unresolved: 0, tags: 0, attachments: 0};
  const v = setup({vaultStatus: async () => ({...s})});
  v.dirty = false; v.status = s;
  await v.refreshStatus();
  assert.equal(v.status, s, 'identical fields must not replace the object');
});

test('own save skips the backlinks reload and the note re-read when links are unchanged', async () => {
  let backlinks = 0, reads = 0;
  const v = setup({
    vaultBacklinks: async () => {backlinks += 1; return [];},
    vaultNote: async (_ws: string, _id: number, p: string) => {reads += 1; return note(p);},
  });
  v.status = {generation: '1', scan_state: 'idle', last_scan_at: 'x', notes: 1, links: 0, unresolved: 0};
  assert.equal(await v.saveNow(), true);
  await new Promise((r) => setTimeout(r, 0));
  assert.equal(backlinks, 0, 'saving X cannot change X\'s incoming links');
  assert.equal(reads, 0, 'the PUT already returned the meta');
});

test('own save re-reads the note once when its link set changed', async () => {
  let reads = 0;
  const v = setup({vaultNote: async (_ws: string, _id: number, p: string) => {reads += 1; return note(p);}});
  v.draft = 'now links [[b]]';
  assert.equal(await v.saveNow(), true);
  await new Promise((r) => setTimeout(r, 0));
  assert.equal(reads, 1);
});

test('S18-20: a poll read issued before an autosave never reverts the editor after it', async () => {
  const slow = deferred<unknown>();
  let reads = 0;
  const v = setup({vaultNote: (_ws: string, _id: number, p: string) => (++reads === 1 ? slow.promise : Promise.resolve(note(p, 'saved text')))});
  v.dirty = false; v.draft = 'saved text'; v.note = note('a.md', 'pre-save text');
  const polling = v.refreshOpenNote(); // GET issued while the buffer is clean
  v.draft = 'saved text'; v.dirty = true;
  assert.equal(await v.saveNow(), true); // the autosave lands first
  slow.resolve(note('a.md', 'pre-save text')); await polling; // then the stale GET
  assert.equal(v.draft, 'saved text', 'editor kept the saved text');
  assert.equal(v.conflict, false);
});

test('S18-21: navigation blocked by a conflict says why', async () => {
  const v = setup(); v.conflict = true;
  await v.open('b.md');
  assert.equal(v.notePath, 'a.md');
  assert.equal(warned.length, 1);
  assert.match(warned[0], /Resolve the conflict on a\.md/);
});

test('S18-22: agent-staged text is never written as a recovery draft', async () => {
  const v = setup(); v.dirty = false; v.draft = v.note.raw;
  v.holdAutosave = true;
  v.onDraftChange('agent proposal');
  await new Promise((r) => setTimeout(r, 450)); // past the 400 ms draft debounce
  assert.ok(!stored.some(([, val]) => val.includes('agent proposal')), JSON.stringify(stored));
  v.holdAutosave = false;
});
