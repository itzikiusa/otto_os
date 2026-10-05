import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { componentFunctions } from './componentFunctions.ts';
import { deferred } from './sourceHarness.ts';

// Fast j/k through pods re-keys the drawer while an exec POST is in flight.
// A session that resolves after its ExecView unmounted must be deleted — else
// the daemon keeps an orphaned `kubectl exec -it` PTY per keypress.
const EXEC = new URL('../src/modules/kubernetes/ExecView.svelte', import.meta.url);

function view() {
  const posted = deferred<{ id: string }>();
  const deleted: string[] = [];
  const state: Record<string, any> = {
    canExec: true, opening: false, isProd: false, error: '', sessionId: null, status: null,
    container: '', clusterId: 'c', ns: 'default', pod: 'p', alive: true,
    ws: { currentId: 'w1' }, NO_WORKSPACE: 'x',
    k8sApi: { exec: () => posted.promise },
    api: { del: async (url: string) => { deleted.push(url); } },
  };
  return { fns: componentFunctions(EXEC, ['open', 'close'], state), posted, deleted };
}

test('an exec session resolved after unmount is deleted, not adopted', async () => {
  const { fns, posted, deleted } = view();
  const opening = fns.open();
  // Unmount: the effect cleanup flips `alive` and runs close() (no id yet).
  fns.alive = false;
  await fns.close();
  assert.deepEqual(deleted, []);
  posted.resolve({ id: 's-late' });
  await opening;
  await new Promise((r) => setTimeout(r, 0));
  assert.deepEqual(deleted, ['/sessions/s-late']);
  assert.equal(fns.sessionId, null);
});

test('a mounted view adopts the session', async () => {
  const { fns, posted, deleted } = view();
  const opening = fns.open();
  posted.resolve({ id: 's1' });
  await opening;
  assert.equal(fns.sessionId, 's1');
  assert.deepEqual(deleted, []);
});

test('autoExec is one-shot: openRow clears it unless the call asks for a shell', () => {
  const src = readFileSync(new URL('../src/modules/kubernetes/ClusterWorkspace.svelte', import.meta.url), 'utf8');
  assert.match(src, /function openRow\(r: K8sRow, tab\?: K8sDrawerTab, exec = false\): void \{\s*autoExec = exec;/);
  // Nothing else turns it on.
  assert.equal(src.match(/autoExec = true/g), null);
});
