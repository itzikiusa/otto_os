// Floating bar pure helpers (node:test, Node's built-in type stripping — the
// `.ts` extension on the import is required in strip mode).
import { test } from 'node:test';
import assert from 'node:assert/strict';
import {
  BAR_MAX_H,
  MAX_THREAD,
  SPACE_COUNT,
  barKeyAction,
  barPresence,
  barWindowHeight,
  buildRows,
  defaultSpaces,
  hitRoute,
  looksLikeCommand,
  moveSelection,
  panelBudget,
  parseBarPref,
  parseSpaces,
  patchTurn,
  pushTurn,
  serializeSpaces,
  spaceLabel,
  type BarTurn,
  type KeyLike,
  type PresenceInput,
} from '../src/lib/floatingBar.ts';
import { frecencyBoost, rankCommands, recordUsage, loadFrecency, FRECENCY_KEY } from '../src/lib/commandSearch.ts';

const NOW = 1_800_000_000_000;
const DAY = 86_400_000;

const cmds = [
  { id: 'core.go-home', title: 'Go to Home', group: 'Navigate', keywords: 'dashboard' },
  { id: 'core.go-git', title: 'Go to Git', group: 'Navigate', keywords: 'repos prs' },
  { id: 'core.new-session', title: 'New Session', group: 'Sessions', keywords: 'spawn agent terminal' },
  { id: 'session.a', title: 'Focus Session: fix tests', group: 'Sessions' },
  { id: 'core.update-clis', title: 'Update all CLIs', group: 'Tools' },
];

// ─── ranking ───────────────────────────────────────────────────────────────

test('ranking: a title match beats a keyword-only match', () => {
  const r = rankCommands(cmds, 'git', {}, NOW);
  assert.equal(r[0].cmd.id, 'core.go-git');
});

test('ranking: no query lists by frecency, most used first, capped', () => {
  const fr = {
    'core.update-clis': { count: 6, lastUsed: NOW - DAY },
    'core.go-home': { count: 1, lastUsed: NOW - 30 * DAY },
  };
  const r = rankCommands(cmds, '  ', fr, NOW, 3);
  assert.equal(r.length, 3);
  assert.deepEqual(r.slice(0, 2).map((x) => x.cmd.id), ['core.update-clis', 'core.go-home']);
});

test('ranking: frecency lifts a used command over an equal fuzzy match', () => {
  const plain = rankCommands(cmds, 'go to', {}, NOW).map((x) => x.cmd.id);
  const used = rankCommands(cmds, 'go to', { 'core.go-git': { count: 8, lastUsed: NOW } }, NOW);
  assert.equal(used[0].cmd.id, 'core.go-git');
  assert.ok(plain.includes('core.go-home'));
});

test('frecency boost: bounded and decays with age', () => {
  assert.equal(frecencyBoost('x', {}, NOW), 0);
  const fresh = frecencyBoost('x', { x: { count: 100, lastUsed: NOW } }, NOW);
  assert.equal(fresh, 20); // 12 (count cap) + 8 (just used)
  const old = frecencyBoost('x', { x: { count: 1, lastUsed: NOW - 30 * DAY } }, NOW);
  assert.equal(old, 1.5);
});

test('frecency persistence survives garbage and blocked storage', () => {
  const store = new Map<string, string>();
  const g = globalThis as { localStorage?: unknown };
  g.localStorage = {
    getItem: (k: string) => store.get(k) ?? null,
    setItem: (k: string, v: string) => void store.set(k, v),
  };
  store.set(FRECENCY_KEY, '[not json');
  assert.deepEqual(loadFrecency(), {});
  recordUsage('core.go-git', NOW);
  recordUsage('core.go-git', NOW + 1);
  assert.deepEqual(loadFrecency()['core.go-git'], { count: 2, lastUsed: NOW + 1 });
  g.localStorage = {
    getItem: () => {
      throw new Error('blocked');
    },
    setItem: () => {
      throw new Error('blocked');
    },
  };
  assert.deepEqual(loadFrecency(), {});
  assert.doesNotThrow(() => recordUsage('x'));
  delete g.localStorage;
});

// ─── rows: command vs Ask Otto ─────────────────────────────────────────────

test('rows: a command-shaped query puts the command first, Ask Otto after', () => {
  const ranked = rankCommands(cmds, 'go to home', {}, NOW).map((r) => r.cmd);
  const rows = buildRows('go to home', ranked, []);
  assert.equal(rows[0].kind, 'cmd');
  assert.equal(rows[0].kind === 'cmd' && rows[0].cmd.id, 'core.go-home');
  assert.ok(rows.some((r) => r.kind === 'ask'));
});

test('rows: free text defaults to Ask Otto even when fuzzy hits exist', () => {
  const q = 'fix the failing tests';
  const ranked = rankCommands(cmds, q, {}, NOW).map((r) => r.cmd);
  const rows = buildRows(q, ranked, []);
  assert.equal(rows[0].kind, 'ask');
  assert.equal(rows[0].kind === 'ask' && rows[0].text, q);
});

test('rows: empty query shows commands only; hits trail with unique keys', () => {
  assert.ok(buildRows('', cmds.slice(0, 2), []).every((r) => r.kind === 'cmd'));
  const rows = buildRows('git', [cmds[1]], [
    { kind: 'repo', id: 'r1', title: 'otto_os' },
    { kind: 'workflow', id: 'w1', title: 'git review' },
  ]);
  assert.deepEqual(rows.map((r) => r.kind), ['cmd', 'ask', 'hit', 'hit']);
  assert.equal(new Set(rows.map((r) => r.key)).size, rows.length);
});

test('looksLikeCommand: word prefixes of the title only', () => {
  assert.equal(looksLikeCommand('new sess', 'New Session'), true);
  assert.equal(looksLikeCommand('focus fix', 'Focus Session: fix tests'), true);
  assert.equal(looksLikeCommand('nwssn', 'New Session'), false);
  assert.equal(looksLikeCommand('Layout: Equal Columns', 'Layout: Equal Columns'), true);
  assert.equal(looksLikeCommand('  :  ', 'New Session'), false);
  assert.equal(looksLikeCommand('', 'New Session'), false);
});

test('selection wraps and handles empty lists', () => {
  assert.equal(moveSelection(0, 1, 3), 1);
  assert.equal(moveSelection(2, 1, 3), 0);
  assert.equal(moveSelection(0, -1, 3), 2);
  assert.equal(moveSelection(-1, -1, 3), 2);
  assert.equal(moveSelection(0, 1, 0), -1);
});

test('search hits map to module routes', () => {
  assert.equal(hitRoute({ kind: 'repo', id: 'abc' }), 'git/abc');
  assert.equal(hitRoute({ kind: 'swarm_task', id: 'x' }), 'swarm');
  assert.equal(hitRoute({ kind: 'weird', id: 'x' }), 'home');
});

// ─── keys ──────────────────────────────────────────────────────────────────

const k = (key: string, m: Partial<KeyLike> = {}): KeyLike => ({
  key,
  metaKey: false,
  ctrlKey: false,
  altKey: false,
  shiftKey: false,
  ...m,
});

test('keys: arrows, Enter, ⌘↵, Esc', () => {
  assert.deepEqual(barKeyAction(k('ArrowDown')), { type: 'next' });
  assert.deepEqual(barKeyAction(k('ArrowUp')), { type: 'prev' });
  assert.deepEqual(barKeyAction(k('Enter')), { type: 'run' });
  assert.deepEqual(barKeyAction(k('Enter', { metaKey: true })), { type: 'ask' });
  assert.deepEqual(barKeyAction(k('Enter', { ctrlKey: true })), { type: 'ask' });
  assert.deepEqual(barKeyAction(k('Escape')), { type: 'escape' });
  assert.equal(barKeyAction(k('Enter', { shiftKey: true })), null);
  assert.equal(barKeyAction(k('ArrowDown', { shiftKey: true })), null); // text selection
  assert.equal(barKeyAction(k('a')), null);
});

test('keys: ⌃1–⌃4 switch spaces; ⌃5, ⌘1 and ⌃⇧1 do not', () => {
  assert.deepEqual(barKeyAction(k('1', { ctrlKey: true })), { type: 'space', index: 0 });
  assert.deepEqual(barKeyAction(k('4', { ctrlKey: true })), { type: 'space', index: 3 });
  assert.equal(barKeyAction(k('5', { ctrlKey: true })), null);
  assert.equal(barKeyAction(k('1', { metaKey: true })), null);
  assert.equal(barKeyAction(k('1', { ctrlKey: true, shiftKey: true })), null);
});

test('keys: IME composition owns every key', () => {
  assert.equal(barKeyAction(k('Enter', { isComposing: true })), null);
  assert.equal(barKeyAction(k('Escape', { isComposing: true })), null);
});

// ─── spaces persistence ────────────────────────────────────────────────────

const turn = (id: string, extra: Partial<BarTurn> = {}): BarTurn => ({
  id,
  q: `q${id}`,
  a: `a${id}`,
  tone: 'ok',
  at: NOW,
  ...extra,
});

test('spaces: defaults are four named, empty, unpinned', () => {
  const d = defaultSpaces();
  assert.equal(d.active, 0);
  assert.equal(d.spaces.length, SPACE_COUNT);
  assert.deepEqual(d.spaces.map((s) => s.name), ['Personal', 'Work', 'Research', 'Home']);
  assert.ok(d.spaces.every((s) => s.workspaceId === null && s.thread.length === 0));
  assert.deepEqual(['01', '02', '03', '04'], [0, 1, 2, 3].map(spaceLabel));
});

test('spaces: round-trip keeps name/workspace/model/thread, drops transient fields', () => {
  const s = defaultSpaces();
  s.active = 2;
  s.spaces[2] = {
    name: 'Casino',
    workspaceId: 'ws-1',
    provider: 'codex',
    model: 'gpt-5',
    thread: [turn('1', { plan: [{ action: 'broadcast', text: 'x' }], closeIds: ['s1'], route: 'agents/s1' })],
  };
  const back = parseSpaces(serializeSpaces(s));
  assert.equal(back.active, 2);
  assert.equal(back.spaces[2].name, 'Casino');
  assert.equal(back.spaces[2].workspaceId, 'ws-1');
  assert.equal(back.spaces[2].model, 'gpt-5');
  assert.equal(back.spaces[2].thread[0].route, 'agents/s1');
  assert.equal(back.spaces[2].thread[0].plan, undefined);
  assert.equal(back.spaces[2].thread[0].closeIds, undefined);
});

test('spaces: garbage, wrong shapes and out-of-range values normalize', () => {
  assert.deepEqual(parseSpaces(null), defaultSpaces());
  assert.deepEqual(parseSpaces('{oops'), defaultSpaces());
  assert.deepEqual(parseSpaces('42'), defaultSpaces());
  const odd = parseSpaces(
    JSON.stringify({
      active: 9,
      spaces: [{ name: '   ', workspaceId: '', thread: 'nope' }, null, { name: 'x'.repeat(80) }],
    }),
  );
  assert.equal(odd.active, 3);
  assert.equal(odd.spaces.length, SPACE_COUNT);
  assert.equal(odd.spaces[0].name, 'Personal'); // blank → default name
  assert.equal(odd.spaces[0].workspaceId, null);
  assert.deepEqual(odd.spaces[0].thread, []);
  assert.equal(odd.spaces[2].name.length, 24);
  assert.equal(odd.spaces[3].name, 'Home');
});

test('spaces: a turn left pending by a closed bar is marked not run', () => {
  const s = defaultSpaces();
  s.spaces[0].thread = [turn('p', { tone: 'pending', a: '' })];
  const back = parseSpaces(serializeSpaces(s));
  assert.equal(back.spaces[0].thread[0].tone, 'info');
  assert.match(back.spaces[0].thread[0].detail ?? '', /Not run/);
});

test('threads: capped at MAX_THREAD and patched by id', () => {
  let t: BarTurn[] = [];
  for (let i = 0; i < MAX_THREAD + 5; i++) t = pushTurn(t, turn(String(i)));
  assert.equal(t.length, MAX_THREAD);
  assert.equal(t[0].id, '5');
  const p = patchTurn(t, '7', { a: 'changed', tone: 'warn' });
  assert.equal(p.find((x) => x.id === '7')?.a, 'changed');
  assert.equal(p.find((x) => x.id === '8')?.a, 'a8');
});

// ─── in-app presence ───────────────────────────────────────────────────────

const base: PresenceInput = {
  pref: 'auto',
  open: false,
  surface: false,
  workFocus: false,
  scrolling: false,
  overlay: false,
};

test('presence: auto docks off Home and while working, is full when open or on Home', () => {
  assert.equal(barPresence(base), 'dock');
  assert.equal(barPresence({ ...base, surface: true }), 'full');
  assert.equal(barPresence({ ...base, scrolling: true }), 'dock');
  assert.equal(barPresence({ ...base, workFocus: true }), 'dock');
  assert.equal(barPresence({ ...base, workFocus: true, surface: true }), 'dock');
  assert.equal(barPresence({ ...base, open: true, workFocus: true }), 'full');
});

test('presence: overlays hide it; prefs pin, dock or turn it off', () => {
  assert.equal(barPresence({ ...base, overlay: true, open: true }), 'away');
  assert.equal(barPresence({ ...base, pref: 'hidden', open: true }), 'off');
  assert.equal(barPresence({ ...base, pref: 'docked' }), 'dock');
  assert.equal(barPresence({ ...base, pref: 'docked', open: true }), 'full');
  assert.equal(barPresence({ ...base, pref: 'pinned' }), 'full');
  assert.equal(barPresence({ ...base, pref: 'pinned', scrolling: true }), 'full');
  assert.equal(barPresence({ ...base, pref: 'pinned', workFocus: true }), 'rest');
  assert.equal(parseBarPref('pinned'), 'pinned');
  assert.equal(parseBarPref('bogus'), 'auto');
  assert.equal(parseBarPref(null), 'auto');
});

// ─── geometry ──────────────────────────────────────────────────────────────

test('geometry: panel budget clamps to the window top and the 560px cap', () => {
  assert.equal(panelBudget(700, 46), BAR_MAX_H - 46);
  assert.equal(panelBudget(200, 46), 188); // 200 - 12px top margin
  assert.equal(panelBudget(5, 46), 0); // never negative
  assert.equal(barWindowHeight(56, 0), 56);
  assert.equal(barWindowHeight(56, 120.2), 177);
  assert.equal(barWindowHeight(56, 5000), BAR_MAX_H);
});

test('ranking: "git" puts Go to Git above Open Git panel (whole-word Navigate hit)', () => {
  const more = [
    { id: 'core.git-panel', title: 'Open Git panel', group: 'View' },
    { id: 'core.side-git', title: 'Open Git side by side', group: 'View', keywords: 'Build' },
    ...cmds,
  ];
  const r = rankCommands(more, 'git', {}, NOW);
  assert.equal(r[0].cmd.id, 'core.go-git');
});
