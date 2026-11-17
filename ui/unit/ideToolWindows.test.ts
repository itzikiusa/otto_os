// IDE tool windows (round 5, I1-11/14/15/23): the tree's git overlay keys, which dock tabs
// show, the reveal-active-file folder expansion and the explorer auto-hide rule.
import assert from 'node:assert/strict';
import { test } from 'node:test';
import { loadSource } from './sourceHarness.ts';

const overlay = loadSource(new URL('../src/modules/ide/git/ideGitTreeOverlay.ts', import.meta.url), {});

test('I1-11: a status for a repo reported as /private/var/… marks the tree row spelled /var/… (and the reverse)', () => {
  const o = overlay.buildIdeTreeGitOverlay([
    { path: '/private/var/folders/x/T/proj', aliases: [], changes: [{ path: 'notes/roadmap.md', kind: 'modified' }, { path: 'new.md', kind: 'untracked' }] },
  ]);
  assert.equal(overlay.ideTreeGitLetter(o, '/var/folders/x/T/proj/notes/roadmap.md'), 'M');
  assert.equal(overlay.ideTreeGitLetter(o, '/private/var/folders/x/T/proj/notes/roadmap.md'), 'M');
  assert.equal(overlay.ideTreeGitLetter(o, '/private/var/folders/x/T/proj/new.md'), 'U');
  assert.equal(overlay.ideTreeGitLetter(o, '/private/var/folders/x/T/proj/clean.go'), null, 'loaded repo, no change: clean');
  assert.equal(overlay.ideTreeGitLetter(o, '/elsewhere/a.go'), undefined, 'no loaded status covers it');
  assert.ok(overlay.ideTreeDirIsDirty(o, '/private/var/folders/x/T/proj/notes/'));
  assert.ok(overlay.ideTreeDirIsDirty(o, '/var/folders/x/T/proj'));
  assert.ok(!overlay.ideTreeDirIsDirty(o, '/var/folders/x/T'));
});

test('I1-11: a symlinked project root is keyed in the tree spelling too', () => {
  const o = overlay.buildIdeTreeGitOverlay([
    { path: '/Volumes/d/code/app', aliases: ['/Users/me/code/app', '/Volumes/d/code/app'], changes: [{ path: 'app/main.go', kind: 'modified' }] },
  ]);
  assert.equal(overlay.ideTreeGitLetter(o, '/Users/me/code/app/app/main.go'), 'M');
  assert.equal(overlay.ideTreeGitLetter(o, '/Volumes/d/code/app/app/main.go'), 'M');
  // A project root opened at a folder INSIDE the repo, through a link: the repo's folder name finds the toplevel.
  assert.equal(overlay.treeAliasRoot('/Volumes/d/code/app', '/Users/me/code/app/services'), '/Users/me/code/app');
  // Same spelling (inside / above / equal): the canonical key already covers it.
  assert.equal(overlay.treeAliasRoot('/repo', '/repo/sub'), null);
  assert.equal(overlay.treeAliasRoot('/repo/sub', '/repo'), null);
  assert.equal(overlay.treeAliasRoot('/private/tmp/r', '/tmp/r'), null);
});

test('I1-11: a status not loaded yet leaves the row to the listing mark; the strongest letter wins', () => {
  const o = overlay.buildIdeTreeGitOverlay([
    { path: '/r', aliases: [], changes: null },
    { path: '/s', aliases: [], changes: [{ path: 'a', kind: 'untracked' }, { path: 'a', kind: 'conflicted' }] },
  ]);
  assert.equal(overlay.ideTreeGitLetter(o, '/r/x'), undefined);
  assert.equal(overlay.ideTreeGitLetter(o, '/s/a'), 'C');
});

const dock = loadSource(new URL('../src/modules/ide/ideDockTabs.svelte.ts', import.meta.url), {});
const facts = (o: Partial<{ hidden: string[]; shown: string[]; used: string[]; hasContent: boolean; active: boolean }> = {}) => ({
  hidden: [], shown: [], used: [], hasContent: false, active: false, ...o,
});

test('I1-14: a fresh dock offers Terminal, Problems, Git, Run — in that order', () => {
  const all = ['debug', 'run', 'git', 'problems', 'usages', 'dbwatch', 'terminal'].map((id) => ({ id }));
  const shown = dock.ideDockOrder(all).filter((t: { id: string }) => dock.ideDockTabVisible(t.id, facts()));
  assert.deepEqual([...shown].map((t: { id: string }) => t.id), ['terminal', 'problems', 'git', 'run']);
});

test('I1-14: Debug / Usages / DB Watch show while in use, once used, or when shown by hand; a hide wins', () => {
  for (const id of ['debug', 'usages', 'dbwatch']) {
    assert.equal(dock.ideDockTabVisible(id, facts()), false, `${id} hidden until used`);
    assert.equal(dock.ideDockTabVisible(id, facts({ hasContent: true })), true, `${id} with content`);
    assert.equal(dock.ideDockTabVisible(id, facts({ used: [id] })), true, `${id} opened this session`);
    assert.equal(dock.ideDockTabVisible(id, facts({ shown: [id] })), true, `${id} shown from ⋯`);
    assert.equal(dock.ideDockTabVisible(id, facts({ hidden: [id], hasContent: true })), false, `${id} hidden by hand`);
    assert.equal(dock.ideDockTabVisible(id, facts({ hidden: [id], active: true })), true, `${id} open tab always shows`);
  }
  assert.equal(dock.ideDockTabVisible('git', facts({ hidden: ['git'] })), false);
  assert.equal(dock.ideDockTabVisible('git', facts()), true);
});

test('I1-14: the explicit "show" choice persists, used tabs are per session, reset clears both', () => {
  const m = dock.ideDockTabMemory;
  m.setShown('usages', true);
  m.markUsed('debug');
  m.markUsed('debug');
  assert.deepEqual([...m.shown], ['usages']);
  assert.deepEqual([...m.used], ['debug']);
  m.setShown('usages', false);
  assert.deepEqual([...m.shown], []);
  m.reset();
  assert.deepEqual([...m.used], []);
});

function room() {
  const toasts: string[] = [];
  const mod = loadSource(new URL('../src/modules/ide/ideExplorerRoom.ts', import.meta.url), {
    '../../lib/toast.svelte': { toasts: { info: (t: string, b?: string) => toasts.push(`${t} — ${b}`) } },
    './shell/ideShortcuts': { ideKeymapPresetOf: (p: unknown) => p, ideShortcutLabel: () => '⌘1' },
  });
  return { mod, toasts };
}

test('I1-15: at 1280 the agent opening hides the explorer and its closing brings it back', () => {
  const { mod } = room();
  const room5 = new mod.IdeExplorerRoom();
  const r = { step: (v: boolean, f: object) => ({ ...room5.step(v, f) }) };
  const base = { width: 1280, treeW: 270, sideW: 0, sideOpen: false };
  assert.deepEqual(r.step(true, base), { visible: true, hid: false });
  // 1280 − 270 − 360 = 650 < 720: make room.
  assert.deepEqual(r.step(true, { ...base, sideW: 360, sideOpen: true }), { visible: false, hid: true });
  assert.deepEqual(r.step(false, { ...base, sideW: 360, sideOpen: true }), { visible: false, hid: false });
  assert.deepEqual(r.step(false, base), { visible: true, hid: false }, 'restored on close');
  // Wide enough: nothing moves.
  assert.deepEqual(r.step(true, { ...base, width: 1700, sideW: 360, sideOpen: true }), { visible: true, hid: false });
});

test('I1-15: a person who toggles the explorer meanwhile keeps their choice', () => {
  const { mod } = room();
  const r = new mod.IdeExplorerRoom();
  const open = { width: 1280, treeW: 270, sideW: 360, sideOpen: true };
  const closed = { ...open, sideW: 0, sideOpen: false };
  r.step(true, closed);
  assert.equal(r.step(true, open).visible, false);
  assert.equal(r.step(true, open).visible, true, 'shown again with the shortcut: respected');
  assert.equal(r.step(true, { ...open, width: 1200 }).visible, true, 'a resize does not hide it again');
  assert.equal(r.step(false, open).visible, false);
  assert.equal(r.step(false, closed).visible, false, 'hidden by hand: not restored');
  assert.equal(mod.ideEditorCramped({ width: 0, treeW: 270, sideW: 360, sideOpen: true }), false, 'unmeasured page');
});

test('I1-15: the hint names the shortcut, once', () => {
  const { mod, toasts } = room();
  mod.hintExplorerYield('jetbrains');
  mod.hintExplorerYield('jetbrains');
  assert.deepEqual(toasts, ['Explorer hidden to make room — Press ⌘1 to show it again.']);
});
