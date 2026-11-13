import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readyToRestore, renameBlocked } from '../src/modules/workbench/docGuards.ts';

/** A tab whose `save` behaves like the store's: held while the remote-change
 *  banner is up, otherwise it lands (or fails) after `rounds` polls. */
function store(opts: { remoteChanged?: boolean; fails?: string; rounds?: number } = {}) {
  const tab = { buffer: 'mine', saved: 'old', saving: false, remoteChanged: !!opts.remoteChanged, saveError: null as string | null };
  const saves: boolean[] = [];
  let polls = 0;
  const s = {
    open: { A: tab } as Record<string, typeof tab | undefined>,
    isDirty: (id: string) => id === 'A' && tab.buffer !== tab.saved,
    async save(_id: string, checkpoint: boolean) {
      saves.push(checkpoint);
      if (tab.remoteChanged) return;
      tab.saving = true; // an in-flight save: `save` returned before it lands
    },
  };
  const wait = async () => {
    polls += 1;
    if (polls >= (opts.rounds ?? 1)) {
      tab.saving = false;
      if (opts.fails) tab.saveError = opts.fails;
      else tab.saved = tab.buffer;
    }
  };
  return { s, tab, saves, wait, polls: () => polls };
}

test('a clean tab restores without saving or asking', async () => {
  const { s, tab, saves, wait } = store();
  tab.saved = tab.buffer;
  let asked = 0;
  assert.equal(await readyToRestore(s, 'A', 4, async () => (asked++, false), wait), true);
  assert.deepEqual(saves, []);
  assert.equal(asked, 0);
});

test('unsaved edits are saved as a checkpoint and the in-flight save is awaited', async () => {
  const { s, tab, saves, wait, polls } = store({ rounds: 3 });
  let asked = 0;
  assert.equal(await readyToRestore(s, 'A', 4, async () => (asked++, false), wait), true);
  assert.deepEqual(saves, [true]);
  assert.equal(polls(), 3);
  assert.equal(tab.saved, 'mine');
  assert.equal(asked, 0);
});

test('a held save (remote-change banner) asks, and No keeps the edits', async () => {
  const { s, saves, wait } = store({ remoteChanged: true });
  const bodies: string[] = [];
  const opts: any[] = [];
  const ok = await readyToRestore(s, 'A', 4, async (body, o) => (bodies.push(body), opts.push(o), false), wait);
  assert.equal(ok, false);
  assert.deepEqual(saves, [], 'a held save is not even attempted');
  assert.match(bodies[0]!, /changed in another window/);
  assert.match(bodies[0]!, /NOT in history/);
  assert.match(bodies[0]!, /revision 4 replaces them/);
  assert.equal(opts[0].danger, true, 'danger keeps the default focus on Cancel');
  assert.equal(opts[0].cancelLabel, 'Keep my edits');
});

test('a failed save asks with the reason, and only an explicit yes proceeds', async () => {
  const { s, wait } = store({ fails: 'disk full' });
  let body = '';
  assert.equal(await readyToRestore(s, 'A', 2, async (b) => ((body = b), true), wait), true);
  assert.match(body, /Saving failed \(disk full\)/);
});

test('rename: applies when the name is unchanged even though the meta object was replaced', () => {
  const started = { id: 'A', name: 'q.sql', rev: 1 };
  const refreshed = { ...started, rev: 2 }; // autosave replaced the element
  assert.notEqual(refreshed, started);
  assert.equal(renameBlocked(refreshed, started.name), null);
});

test('rename: says why when the file was renamed or removed underneath', () => {
  assert.match(renameBlocked({ name: 'other.sql' }, 'q.sql')!, /renamed to “other\.sql”/);
  assert.match(renameBlocked(undefined, 'q.sql')!, /deleted or moved/);
});
