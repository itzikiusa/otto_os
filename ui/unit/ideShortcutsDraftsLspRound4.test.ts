// IDE round 4: draft storage stays inside a bounded slice of the shared localStorage
// (G12), bare ⌃ shortcuts stay out of text fields and ⌘L / ⌘G mean one thing each
// (G6/G7), language-server sockets reattach after they drop (G10), and a git-triggered
// reload or a Compare apply never replaces unsaved edits silently (G2/G3).
import assert from 'node:assert/strict';
import { test } from 'node:test';
import { createHash } from 'node:crypto';
import { deferred, loadSource } from './sourceHarness.ts';
import {
  IDE_UNDO_DRAFT_INDEX_KEY,
  IDE_UNDO_DRAFT_PREFIX,
  MAX_DRAFT_TOTAL_CHARS,
  MAX_JOURNAL_ENTRY_CHARS,
  readIdeUndoDraft,
  writeIdeUndoDraftImmediate,
} from '../src/lib/components/cm-ide-drafts.ts';

class MemStore {
  map = new Map<string, string>();
  quotaChars = Infinity;
  getItem(k: string) { return this.map.get(k) ?? null; }
  setItem(k: string, v: string) {
    const used = [...this.map.entries()].reduce((n, [kk, vv]) => n + (kk === k ? 0 : vv.length), 0);
    if (used + v.length > this.quotaChars) throw new DOMException('quota', 'QuotaExceededError');
    this.map.set(k, v);
  }
  removeItem(k: string) { this.map.delete(k); }
  draftChars() {
    let n = 0;
    for (const [k, v] of this.map) if (k.startsWith(IDE_UNDO_DRAFT_PREFIX)) n += v.length;
    return n;
  }
}

// ── G12 ─────────────────────────────────────────────────────────────────────

test('G12: drafts stay inside their total budget, evicting the oldest first', async () => {
  const store = new MemStore();
  const big = 'x'.repeat(100_000);
  for (let i = 0; i < 12; i++) {
    writeIdeUndoDraftImmediate(`/repo/f${i}.ts`, big, `${big}${i}`, store);
    await new Promise((r) => setTimeout(r, 2));
  }
  assert.ok(store.draftChars() <= MAX_DRAFT_TOTAL_CHARS, `drafts use ${store.draftChars()} chars`);
  assert.ok(readIdeUndoDraft('/repo/f11.ts', store), 'the newest draft is kept');
  assert.equal(readIdeUndoDraft('/repo/f0.ts', store), null, 'the oldest draft went first');
  const idx = JSON.parse(store.getItem(IDE_UNDO_DRAFT_INDEX_KEY)!);
  for (const p of Object.keys(idx)) assert.ok(store.getItem(`${IDE_UNDO_DRAFT_PREFIX}${p}`), `index entry ${p} has its draft`);
});

test('G12: a full origin quota never throws out of a draft write and keeps the newest draft', () => {
  const store = new MemStore();
  writeIdeUndoDraftImmediate('/repo/old.ts', 'a'.repeat(30_000), 'b'.repeat(30_000), store);
  store.map.set('otto.other.module', 'z'.repeat(40_000));
  store.quotaChars = 110_000;
  assert.doesNotThrow(() => writeIdeUndoDraftImmediate('/repo/new.ts', 'c'.repeat(30_000), 'd'.repeat(30_000), store));
  assert.ok(readIdeUndoDraft('/repo/new.ts', store));
  assert.equal(store.getItem('otto.other.module')?.length, 40_000, 'other modules\' data is untouched');
});

test('G12: a file too large for a draft drops its older, now stale draft', () => {
  const store = new MemStore();
  writeIdeUndoDraftImmediate('/repo/big.ts', 'v1', 'v1 edited', store);
  const huge = 'y'.repeat(MAX_JOURNAL_ENTRY_CHARS + 1);
  writeIdeUndoDraftImmediate('/repo/big.ts', huge, `${huge}!`, store);
  assert.equal(readIdeUndoDraft('/repo/big.ts', store), null);
});

// ── G6 / G7 ─────────────────────────────────────────────────────────────────

class FakeEl {
  tagName: string;
  isContentEditable: boolean;
  classes: string[];
  parent: FakeEl | null;
  children: FakeEl[] = [];
  constructor(tagName: string, classes: string[] = [], parent: FakeEl | null = null, editable = false) {
    this.tagName = tagName; this.classes = classes; this.parent = parent; this.isContentEditable = editable;
    parent?.children.push(this);
  }
  matches(sel: string) { return sel.split(',').some((s) => this.classes.includes(s.trim().replace(/^\./, ''))); }
  closest(sel: string): FakeEl | null { for (let e: FakeEl | null = this; e; e = e.parent) if (e.matches(sel)) return e; return null; }
  querySelector(sel: string): FakeEl | null {
    for (const c of this.children) { if (c.matches(sel)) return c; const d = c.querySelector(sel); if (d) return d; }
    return null;
  }
}

function shellModule() {
  const calls: string[] = [];
  const ide = {
    activePath: '/repo/a.ts', cursorInfo: { line: 1, col: 1 }, searchEverywhereOpen: false, activeDebug: null,
    startRun: async () => { calls.push('run'); }, startDebug: async () => { calls.push('debug'); },
    goToLineOpen: false, jumpTo: () => {}, openSettings: () => {}, saveFile: async () => true,
  };
  const mod = loadSource(new URL('../src/modules/ide/shell/ideShellCommands.ts', import.meta.url), {
    '../../../lib/confirm.svelte': { confirmer: { promptText: async () => { calls.push('goToLine'); return null; } } },
    '../../../lib/contextmenu.svelte': { ctxMenu: { showAt: () => {} } },
    '../../../lib/idePreferences.svelte': { readIdePreferences: () => ({}), writeIdePreferences: () => {} },
    '../ideStore.svelte': { ide, ideBasename: (p: string) => p },
    '../store/ideAgentController.svelte': { ideAgent: { markSelectionFromEditor: () => { calls.push('agent'); return null; }, openInlineEdit: () => {} } },
  }, { Element: FakeEl, window: { location: { reload() {} }, dispatchEvent() {} }, CustomEvent: class {} });
  return { mod, calls };
}

function key(target: FakeEl, k: string, mods: { ctrl?: boolean; meta?: boolean; shift?: boolean; alt?: boolean } = {}) {
  let prevented = false;
  return {
    target, key: k, code: k.length === 1 ? `Key${k.toUpperCase()}` : k,
    ctrlKey: Boolean(mods.ctrl), metaKey: Boolean(mods.meta), shiftKey: Boolean(mods.shift), altKey: Boolean(mods.alt),
    get defaultPrevented() { return prevented; }, preventDefault() { prevented = true; },
  };
}

test('G6: ⌃D / ⌃R typed in a text field edit text instead of starting Debug / Run', () => {
  const { mod, calls } = shellModule();
  const page = new FakeEl('DIV', ['ide-page']);
  const composer = new FakeEl('TEXTAREA', [], page);
  const commit = new FakeEl('INPUT', [], page);
  const findField = new FakeEl('INPUT', [], new FakeEl('DIV', ['cm-search'], new FakeEl('DIV', ['cm-editor'], page)));
  const richText = new FakeEl('DIV', [], page, true);
  for (const t of [composer, commit, findField, richText]) {
    const d = key(t, 'd', { ctrl: true });
    mod.handleIdeWindowKeyDown(d, false, () => {});
    assert.equal(d.defaultPrevented, false, `${t.tagName}: ⌃D stays forward-delete`);
    mod.handleIdeWindowKeyDown(key(t, 'r', { ctrl: true }), false, () => {});
  }
  assert.deepEqual(calls, []);
  mod.handleIdeWindowKeyDown(key(page, 'd', { ctrl: true }), false, () => {});
  mod.handleIdeWindowKeyDown(key(page, 'r', { ctrl: true }), false, () => {});
  assert.deepEqual(calls, ['debug', 'run'], 'outside a text field the shortcuts still work');
});

test('G7: ⌘L adds the selection to the agent; ⌘G is Go to Line unless the find panel is open', () => {
  const { mod, calls } = shellModule();
  const editor = new FakeEl('DIV', ['cm-editor']);
  const content = new FakeEl('DIV', ['cm-content'], editor, true);
  assert.equal(mod.handleIdeChord(key(content, 'l', { meta: true }), false, () => {}), true);
  assert.deepEqual(calls, ['agent']);
  assert.equal(mod.handleIdeChord(key(content, 'g', { meta: true }), false, () => {}), true);
  assert.deepEqual(calls, ['agent', 'goToLine']);
  const panel = new FakeEl('DIV', ['cm-search'], editor);
  const findInput = new FakeEl('INPUT', [], panel);
  const g = key(findInput, 'g', { meta: true });
  assert.equal(mod.handleIdeChord(g, false, () => {}), false, '⌘G is left to "find next"');
  assert.equal(g.defaultPrevented, false);
  assert.equal(mod.inTextField(content), false, 'the code editor itself is not a text field');
});

// ── G10 ─────────────────────────────────────────────────────────────────────

function reconnectModule() {
  const events: { type: string; detail: { rootPath: string; state?: string; attempt?: number } }[] = [];
  const timers: { fn: () => void; ms: number }[] = [];
  const mod = loadSource(new URL('../src/lib/components/cm-ide-lsp-reconnect.ts', import.meta.url), {}, {
    window: { dispatchEvent: (e: { type: string; detail: never }) => { events.push({ type: e.type, detail: e.detail }); return true; } },
    CustomEvent: class { type: string; detail: unknown; constructor(type: string, init: { detail: unknown }) { this.type = type; this.detail = init.detail; } },
    setTimeout: (fn: () => void, ms: number) => { timers.push({ fn, ms }); return timers.length; },
    clearTimeout: () => {},
  });
  const fire = () => { const t = timers.shift(); t?.fn(); return t?.ms; };
  return { mod, events, timers, fire };
}

test('G10: a dropped session reattaches with backoff and reports reconnecting → connected', () => {
  const { mod, events, fire } = reconnectModule();
  mod.noteLspSocketClosed('/repo/', true);
  assert.equal(mod.isLspReconnecting('/repo/svc'), true, 'a module root inside the project counts');
  assert.equal(fire(), 1000);
  assert.deepEqual(events.map((e) => [e.type, e.detail.state ?? null]), [['otto:ide-lsp-link', 'reconnecting'], ['otto:ide-reconnect-lsp', null]]);
  mod.noteLspSocketClosed('/repo', false); // the first attempt failed: the daemon is still down
  assert.equal(fire(), 2000);
  mod.noteLspSessionReady('/repo/svc');
  assert.equal(events.at(-1)?.detail.state, 'connected');
  assert.equal(mod.isLspReconnecting('/repo'), false);
});

test('G10: attempts are bounded, then the editor is told language features are off', () => {
  const { mod, events, fire } = reconnectModule();
  mod.noteLspSocketClosed('/repo', true);
  for (let i = 0; i < mod.LSP_RECONNECT_DELAYS_MS.length; i++) { fire(); mod.noteLspSocketClosed('/repo', false); }
  assert.equal(events.at(-1)?.detail.state, 'lost');
  assert.equal(mod.isLspReconnecting('/repo'), false);
  mod.retryLspNow('/repo');
  assert.equal(events.at(-1)?.type, 'otto:ide-reconnect-lsp', 'Retry dials at once');
});

test('G10: a close of a socket that never had a session (or plain unknown noise) starts nothing', () => {
  const { mod, events, timers } = reconnectModule();
  mod.noteLspSocketClosed('/repo', false);
  assert.equal(events.length, 0);
  assert.equal(timers.length, 0);
});

// ── G3 ──────────────────────────────────────────────────────────────────────

function gitModule(get: () => Promise<unknown>) {
  const tabs = [{ path: '/repo/a.ts', name: 'a.ts', content: 'v1', savedContent: 'v1', dirty: false, sha256: 's1', eol: '\n', loading: false, error: null }];
  const live = new Map<string, string>();
  const replaced: string[] = [];
  const ide = {
    tabs, target: { kind: 'local', root: '/repo' }, projectRoots: [{ path: '/repo', isPrimary: true }], activePath: '/repo/a.ts', splitPath: null,
    resolvePath: (p: string) => p, targetQuery: () => 'root=%2Frepo',
    getBuffer: (p: string) => live.get(p) ?? tabs.find((t) => t.path === p)?.content ?? '',
    replaceBufferContent: async (_p: string, c: string) => { replaced.push(c); return true; },
    updateBuffer: () => {}, closeTab: async () => {},
  };
  const mod = loadSource(new URL('../src/modules/ide/git/ideGitController.svelte.ts', import.meta.url), {
    '../../../lib/api/client': { api: { get }, ApiError: class extends Error { status = 0; } },
    '../../../lib/api/git': { gitApi: {} },
    '../../../lib/stores/git.svelte': { git: { repos: [], allRepos: [], liveRev: {}, statusFor: () => null } },
    '../../../lib/stores/workspace.svelte': { SCRATCH_WORKSPACE_ID: 'scratch', ws: { current: null, workspaces: [] } },
    '../../../lib/toastError': { toastError: () => {} },
    '../ideStore.svelte': { ide, ideBasename: (p: string) => p },
    '../store/ideStateSlices.svelte': { onIdeTargetReset: () => {} },
    '../store/ideTypes': { detectTextEol: () => '\n', diskEolsOf: () => undefined, normalizeEditorLf: (s: string) => s },
  });
  return { ideGit: mod.ideGit, tabs, live, replaced };
}

test('G3: a git-triggered reload does not overwrite keystrokes typed while it was reading', async () => {
  const read = deferred<unknown>();
  const { ideGit, tabs, live, replaced } = gitModule(() => read.promise);
  const done = ideGit.reloadIdeTabIfOpen('/repo/a.ts', { onlyIfChangedOnDisk: true, onlyIfClean: true });
  await new Promise((r) => setTimeout(r, 0));
  live.set('/repo/a.ts', 'v1 typed');
  read.resolve({ content: 'disk', sha256: 's2' });
  await done;
  assert.deepEqual(replaced, []);
  assert.equal(tabs[0].savedContent, 'v1');
  assert.equal(tabs[0].sha256, 's1', 'old sha kept: saving asks about the disk change');
});

test('G3: a background reload skips a tab with unsynced keystrokes', async () => {
  let reads = 0;
  const { ideGit, live } = gitModule(async () => { reads += 1; return { content: 'disk', sha256: 's2' }; });
  live.set('/repo/a.ts', 'v1 typed');
  await ideGit.reloadIdeTabIfOpen('/repo/a.ts', { onlyIfClean: true });
  assert.equal(reads, 0);
});

// ── G2 ──────────────────────────────────────────────────────────────────────

function compareModule(opts: { tab?: Record<string, unknown> | null; live?: string; askAnswer?: boolean }) {
  const calls: string[] = [];
  const puts: { content: string; expected_sha256?: string }[] = [];
  const tabs = opts.tab ? [opts.tab] : [];
  const ide = {
    tabs, target: { kind: 'local', root: '/repo' }, targetPayload: { kind: 'local', root: '/repo' },
    resolvePath: (p: string) => p,
    getBuffer: (p: string) => (p === opts.tab?.path ? opts.live ?? (opts.tab?.content as string) : ''),
    applyAgentEdit: async (_p: string, c: string) => { calls.push(`edit:${c}`); return true; },
    saveFile: async (p: string) => { calls.push(`save:${p}`); return true; },
    openFile: async () => {},
  };
  const types = loadSource(new URL('../src/modules/ide/store/ideTypes.ts', import.meta.url), {
    '../../../lib/api/client': { getToken: () => '', laneFetch: () => { throw new Error('unused'); } },
  }, { TextEncoder });
  const mod = loadSource(new URL('../src/modules/ide/history/ideComparisonLoader.ts', import.meta.url), {
    '../../../lib/api/client': { api: { put: async (_u: string, b: { content: string; expected_sha256?: string }) => { puts.push(b); return { sha256: 'n' }; } }, ApiError: class extends Error { status = 0; } },
    '../../../lib/api/git': { gitApi: {} },
    '../../../lib/components/cm-ide-history': { getLatestClipboardForCompare: () => '' },
    '../../../lib/components/cm-ide-prefs': { readIdePreferences: () => ({ localHistoryEnabled: false, localHistoryRetentionDays: 7 }) },
    '../../../lib/confirm.svelte': { confirmer: { ask: async () => { calls.push('ask'); return opts.askAnswer ?? false; } } },
    '../../../lib/toast.svelte': { toasts: { success() {}, error: (t: string) => calls.push(`err:${t}`) } },
    '../../../lib/toastError': { toastError: (t: string) => calls.push(`err:${t}`) },
    '../git/ideGitController.svelte': { ideGit: { rootContainingPath: () => null, refreshActive: async () => {} } },
    '../ideStore.svelte': { ide, ideBasename: (p: string) => p.split('/').pop() },
    '../store/ideTypes': types,
  });
  return { mod, calls, puts };
}

const openTab = (content: string, savedContent: string) => ({ path: '/repo/a.ts', name: 'a.ts', content, savedContent, loading: false, error: null, readOnly: false, truncated: false });

test('G2: applying a hunk to a tab with unsaved edits asks first, and does nothing when declined', async () => {
  const { mod, calls, puts } = compareModule({ tab: openTab('saved', 'saved'), live: 'saved + mine', askAnswer: false });
  assert.equal(await mod.writeCurrentAndSyncIde('/repo/a.ts', 'hunk applied', false, 'saved'), null);
  assert.deepEqual(calls, ['ask']);
  assert.equal(puts.length, 0);
});

test('G2: applying to an open tab is one buffer edit plus the normal guarded save, not a raw PUT', async () => {
  const { mod, calls, puts } = compareModule({ tab: openTab('saved', 'saved') });
  assert.equal(await mod.writeCurrentAndSyncIde('/repo/a.ts', 'hunk applied', false, 'saved'), 'hunk applied');
  assert.deepEqual(calls, ['edit:hunk applied', 'save:/repo/a.ts']);
  assert.equal(puts.length, 0);
});

test('G2: a file that is not open is written with the compared disk text as precondition and its CRLF endings', async () => {
  const { mod, puts } = compareModule({ tab: null });
  const disk = 'a\r\nb\r\n';
  assert.equal(await mod.writeCurrentAndSyncIde('/repo/a.ts', 'a\nc\n', false, disk), 'a\r\nc\r\n');
  assert.equal(puts[0].content, 'a\r\nc\r\n');
  assert.equal(puts[0].expected_sha256, createHash('sha256').update(disk).digest('hex'));
});

test('G10: an eviction with reason "shutdown" does not retry; it reattaches once the app reconnects', () => {
  const { mod, events, timers } = reconnectModule();
  let resume: (() => void) | null = null;
  let unsubscribed = 0;
  mod.noteLspDaemonShutdown('/repo', true, (r: () => void) => { resume = r; return () => { unsubscribed += 1; }; });
  assert.equal(timers.length, 0, 'no backoff timer against a daemon going down');
  assert.equal(events.at(-1)?.detail.state, 'waiting');
  assert.ok(resume, 'waits on the app connection');
  (resume as unknown as () => void)();
  assert.equal(events.at(-1)?.type, 'otto:ide-reconnect-lsp', 'reattaches when the daemon is back');
  assert.equal(unsubscribed, 1);
  (resume as unknown as () => void)();
  assert.equal(events.filter((e) => e.type === 'otto:ide-reconnect-lsp').length, 1, 'only once');
});
