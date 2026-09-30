import { test } from 'node:test';
import assert from 'node:assert/strict';
import { loadSource } from './sourceHarness.ts';

// ⌘F routing (src/lib/keys.ts routeFind): a focused owner wins, then the
// highest-ranked on-screen session terminal, then the page-wide find — and a
// text field / editor / modal outside the terminal keeps ⌘F on the page.

type Focus = { tagName: string; isContentEditable?: boolean; in?: string[] };

function load(opts: { focus?: Focus | null; modal?: boolean } = {}) {
  const f = opts.focus ?? null;
  const active = f && {
    tagName: f.tagName,
    isContentEditable: !!f.isContentEditable,
    closest: (sel: string) => ((f.in ?? []).some((c) => sel.includes(c)) ? {} : null),
  };
  const document = {
    activeElement: active,
    querySelector: () => (opts.modal ? {} : null),
  };
  return loadSource(new URL('../src/lib/keys.ts', import.meta.url), {}, { document });
}

function owner(rank: number, log: string[], name: string) {
  return { rank: () => rank, open: () => log.push(name) };
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
  const k = load({ focus: { tagName: 'BODY' }, modal: true });
  const log: string[] = [];
  k.registerFindOwner(owner(2, log, 'pane'));
  k.routeFind(() => log.push('page'));
  assert.deepEqual(log, ['page']);
});
