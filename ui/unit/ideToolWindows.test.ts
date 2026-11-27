// IDE tool windows (round 5, I1-11/14/23): the tree's git overlay keys, which dock tabs
// show and the reveal-active-file folder expansion. (The explorer auto-hide rule, I1-15,
// went with the shared list pane: ListDetail clamps it so the editor keeps its room.)
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
