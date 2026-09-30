import { test } from 'node:test';
import assert from 'node:assert/strict';
import { loadSource } from './sourceHarness.ts';

// ⌘F routing (src/lib/keys.ts routeFind): a focused owner wins, then the
// highest-ranked on-screen session terminal, then the page-wide find — and a
// text field / editor / modal outside the terminal keeps ⌘F on the page.

type Focus = { tagName: string; isContentEditable?: boolean; in?: string[] };

function load(opts: { focus?: Focus | null; dialogs?: { visible: boolean }[] } = {}) {
  const f = opts.focus ?? null;
  const active = f && {
    tagName: f.tagName,
    isContentEditable: !!f.isContentEditable,
    closest: (sel: string) => ((f.in ?? []).some((c) => sel.includes(c)) ? {} : null),
  };
  const dialogs = (opts.dialogs ?? []).map((d) => ({ checkVisibility: () => d.visible }));
  const document = {
    activeElement: active,
    querySelectorAll: () => dialogs,
  };
  return loadSource(new URL('../src/lib/keys.ts', import.meta.url), {}, { document });
}

/** A fake element: `path` is its position in the tree (e.g. [0, 2]); `cls`
 *  are classes on it or an ancestor (what closest() finds). */
function node(path: number[], cls: string[] = []) {
  return {
    path,
    closest: (sel: string) => (cls.some((c) => sel.includes(c)) ? {} : null),
    contains: (o: { path: number[] }) => o.path.length >= path.length && path.every((p, i) => o.path[i] === p),
  };
}

const PANE = node([0]);

function owner(rank: number, log: string[], name: string, pane: unknown = PANE) {
  return { rank: () => rank, pane: () => pane, open: () => log.push(name) };
}

test('no owners → page find', () => {
  const k = load();
  const log: string[] = [];
  k.routeFind(() => log.push('page'));
  assert.deepEqual(log, ['page']);
});

test('focus-owned openFind beats any pane owner', () => {
  const k = load({ focus: { tagName: 'TEXTAREA', in: ['.cm-editor'] } });
  const log: string[] = [];
  k.registerFindOwner(owner(2, log, 'pane'));
  k.keyContext.openFind = () => log.push('editor');
  k.routeFind(() => log.push('page'));
  assert.deepEqual(log, ['editor']);
});

test('nothing focused → highest-ranked visible terminal', () => {
  const k = load({ focus: { tagName: 'BODY' } });
  const log: string[] = [];
  k.registerFindOwner(owner(1, log, 'same-session'));
  k.registerFindOwner(owner(2, log, 'focused-pane'));
  k.registerFindOwner(owner(0, log, 'hidden'));
  k.routeFind(() => log.push('page'));
  assert.deepEqual(log, ['focused-pane']);
});

test('a header button holding focus still routes to the pane', () => {
  const k = load({ focus: { tagName: 'BUTTON' } });
  const log: string[] = [];
  k.registerFindOwner(owner(1, log, 'pane'));
  k.routeFind(() => log.push('page'));
  assert.deepEqual(log, ['pane']);
});

test('only rank-0 (parked / hidden) owners → page find', () => {
  const k = load();
  const log: string[] = [];
  k.registerFindOwner(owner(0, log, 'parked'));
  k.routeFind(() => log.push('page'));
  assert.deepEqual(log, ['page']);
});

test('unregistered owner no longer takes ⌘F', () => {
  const k = load();
  const log: string[] = [];
  const off = k.registerFindOwner(owner(2, log, 'pane'));
  off();
  k.routeFind(() => log.push('page'));
  assert.deepEqual(log, ['page']);
});

test('a text field / contenteditable outside the terminal keeps ⌘F on the page', () => {
  for (const focus of [{ tagName: 'INPUT' }, { tagName: 'DIV', isContentEditable: true }, { tagName: 'DIV', in: ['.cm-editor'] }]) {
    const k = load({ focus });
    const log: string[] = [];
    k.registerFindOwner(owner(2, log, 'pane'));
    k.routeFind(() => log.push('page'));
    assert.deepEqual(log, ['page'], JSON.stringify(focus));
  }
});

test('an xterm textarea is not "a text field elsewhere"', () => {
  const k = load({ focus: { tagName: 'TEXTAREA', in: ['.xterm'] } });
  const log: string[] = [];
  k.registerFindOwner(owner(1, log, 'pane'));
  k.routeFind(() => log.push('page'));
  assert.deepEqual(log, ['pane']);
});

test('an open modal keeps ⌘F on the page', () => {
  const k = load({ focus: { tagName: 'BODY' }, dialogs: [{ visible: true }] });
  const log: string[] = [];
  k.registerFindOwner(owner(2, log, 'pane'));
  k.routeFind(() => log.push('page'));
  assert.deepEqual(log, ['page']);
});

test('a hidden aria-modal dialog (the compact Drawer kept mounted) does not block the pane', () => {
  const k = load({ focus: { tagName: 'BODY' }, dialogs: [{ visible: false }] });
  const log: string[] = [];
  k.registerFindOwner(owner(2, log, 'pane'));
  k.routeFind(() => log.push('page'));
  assert.deepEqual(log, ['pane']);
});

test('last click inside the pane (header, terminal) → the pane', () => {
  const k = load({ focus: { tagName: 'BODY' } });
  const log: string[] = [];
  k.registerFindOwner(owner(2, log, 'pane'));
  k.noteInteraction(node([0, 1, 3]));
  k.routeFind(() => log.push('page'));
  assert.deepEqual(log, ['pane']);
});

test('last click on content beside the pane (loop timeline) → page find', () => {
  const k = load({ focus: { tagName: 'BODY' } });
  const log: string[] = [];
  k.registerFindOwner(owner(2, log, 'pane'));
  k.noteInteraction(node([1, 0]));
  k.routeFind(() => log.push('page'));
  assert.deepEqual(log, ['page']);
});

test('the right panel never hands ⌘F to the terminal, even nested in the pane tree', () => {
  const k = load({ focus: { tagName: 'BODY' } });
  const log: string[] = [];
  k.registerFindOwner(owner(2, log, 'pane'));
  k.noteInteraction(node([0, 5], ['.rpanel']));
  k.routeFind(() => log.push('page'));
  assert.deepEqual(log, ['page']);
});

test('the click picks WHICH pane: a lower-ranked pane that was clicked beats an unclicked higher one', () => {
  const k = load({ focus: { tagName: 'BODY' } });
  const log: string[] = [];
  k.registerFindOwner(owner(2, log, 'loop-session', node([0])));
  k.registerFindOwner(owner(1, log, 'agents-pane', node([2])));
  k.noteInteraction(node([2, 0]));
  k.routeFind(() => log.push('page'));
  assert.deepEqual(log, ['agents-pane']);
});

test('a find-neutral relay (phone quick-action bar) keeps the last real interaction', () => {
  const k = load({ focus: { tagName: 'BODY' } });
  const log: string[] = [];
  k.registerFindOwner(owner(2, log, 'pane'));
  k.noteInteraction(node([0, 2]));
  k.noteInteraction(node([9], ['[data-find-neutral]']));
  k.routeFind(() => log.push('page'));
  assert.deepEqual(log, ['pane']);
});
