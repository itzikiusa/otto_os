// S14-02: archive kills the PTY, so every USER-FACING archive / restart entry
// point must go through the store's working-guard (`ws.requestArchive`,
// `ws.requestRestart` / `ws.confirmRestart`) — closing a busy tab always
// confirmed, these four paths used to abort a mid-turn agent silently.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { join } from 'node:path';

const read = (p: string): string => readFileSync(join(import.meta.dirname, '..', p), 'utf8');

const ARCHIVE_ENTRY_POINTS = [
  'src/modules/agents/SessionView.svelte',
  'src/modules/agents/history/HistoryPage.svelte',
  'src/modules/home/boxes/ClassroomsBox.svelte',
  'src/shell/TabBar.svelte',
  'src/shell/Navigator.svelte',
  'src/shell/App.svelte',
];

test('every user-facing archive goes through ws.requestArchive', () => {
  for (const f of ARCHIVE_ENTRY_POINTS) {
    const src = read(f);
    assert.doesNotMatch(src, /ws\.archiveSession\(/, `${f} archives without the working-guard`);
    assert.match(src, /ws\.requestArchive\(/, `${f} should use ws.requestArchive`);
  }
});

test('requestArchive confirms a working agent before archiving', () => {
  const store = read('src/lib/stores/workspace.svelte.ts');
  const fn = store.slice(store.indexOf('  async requestArchive(id: Id)'), store.indexOf('  async unarchiveSession('));
  assert.match(fn, /this\.statusMap\[id\] === 'working'/);
  assert.match(fn, /confirmer\.ask\(/);
  assert.match(fn, /if \(!ok\) return false;/);
  assert.ok(fn.indexOf('confirmer.ask') < fn.indexOf('this.archiveSession(id)'), 'confirm before the archive');
});

test('SessionView "Save & restart" runs the restart working-guard first', () => {
  const src = read('src/modules/agents/SessionView.svelte');
  const fn = src.slice(src.indexOf('  async function saveDirs('));
  assert.match(fn, /if \(alsoRestart && !\(await ws\.confirmRestart\(sessionId\)\)\) return;/);
  assert.ok(fn.indexOf('confirmRestart') < fn.indexOf('ws.restartSession'), 'guard before the restart');
});
