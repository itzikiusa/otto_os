// Otto School — the headmaster's actions (kick out / detention / release)
// with fake effects: confirm copy, the one delete / archive / unarchive path,
// the walk animations and the restore on failure or a cancelled guard.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { kickOut, kickOutPrompt, release, sendToDetention } from '../src/modules/home/school/actions.ts';

const kid = {
  id: 'k1',
  title: 'Fix the flaky test',
  workspaceName: 'Otto',
  pose: 'idle' as const,
  canManage: true,
  background: false,
  source: null,
};

function log() {
  const calls: string[] = [];
  return { calls, push: (s: string) => calls.push(s) };
}

test('kick-out confirms with the session, the workspace and no-undo; mid-turn says so', () => {
  const p = kickOutPrompt(kid);
  assert.match(p.message, /Fix the flaky test/);
  assert.match(p.message, /Otto/);
  assert.match(p.message, /no Undo/);
  assert.equal(p.opts.danger, true);
  const busy = kickOutPrompt({ ...kid, pose: 'working' });
  assert.match(busy.message, /mid-turn/);
  assert.equal(busy.opts.title, 'Kick out a working agent');
  assert.match(kickOutPrompt({ ...kid, background: true, source: 'pr_review' }).message, /pr review run/);
});

test('kick-out: confirmed → delete + walk-out together, then a toast', async () => {
  const l = log();
  const ok = await kickOut(kid, {
    ask: async () => true,
    kill: async (id) => void l.push(`kill:${id}`),
    animate: async (id) => void l.push(`walk:${id}`),
    restore: (id) => l.push(`restore:${id}`),
    done: (t) => l.push(`done:${t}`),
    failed: (t) => l.push(`failed:${t}`),
  });
  assert.equal(ok, true);
  assert.deepEqual(l.calls.sort(), ['done:Kicked out Fix the flaky test', 'kill:k1', 'walk:k1']);
});

test('kick-out: cancelled → nothing happens; failed delete → the kid walks back', async () => {
  const l = log();
  assert.equal(await kickOut(kid, { ask: async () => false, kill: async () => void l.push('kill'), done: () => l.push('done'), failed: () => l.push('failed') }), false);
  assert.deepEqual(l.calls, []);
  const ok = await kickOut(kid, {
    ask: async () => true,
    kill: async () => {
      throw new Error('403');
    },
    restore: (id) => l.push(`restore:${id}`),
    done: () => l.push('done'),
    failed: (t) => l.push(`failed:${t}`),
  });
  assert.equal(ok, false);
  assert.deepEqual(l.calls, ['restore:k1', 'failed:Couldn’t kick out “Fix the flaky test”']);
});

test('no actions for a kid you can’t manage', async () => {
  const l = log();
  const viewer = { ...kid, canManage: false };
  assert.equal(await kickOut(viewer, { ask: async () => true, kill: async () => void l.push('kill'), done: () => {}, failed: () => {} }), false);
  assert.equal(await sendToDetention(viewer, { archive: async () => void l.push('archive'), failed: () => {} }), false);
  assert.equal(await release(viewer, { unarchive: async () => void l.push('unarchive'), done: () => {}, failed: () => {} }), false);
  assert.deepEqual(l.calls, []);
});

test('detention archives (with the working hint) while the kid walks to the bench', async () => {
  const l = log();
  const ok = await sendToDetention(
    { ...kid, pose: 'working' },
    {
      archive: async (id, hint) => void l.push(`archive:${id}:${hint.working}`),
      animate: async (id) => void l.push(`walk:${id}`),
      restore: (id) => l.push(`restore:${id}`),
      failed: (t) => l.push(`failed:${t}`),
    },
  );
  assert.equal(ok, true);
  assert.deepEqual(l.calls.sort(), ['archive:k1:true', 'walk:k1']);
});

test('detention: a cancelled guard walks the kid back without an error', async () => {
  const l = log();
  const ok = await sendToDetention(kid, {
    archive: async () => false,
    restore: (id) => l.push(`restore:${id}`),
    failed: (t) => l.push(`failed:${t}`),
  });
  assert.equal(ok, false);
  assert.deepEqual(l.calls, ['restore:k1']);
});

test('release unarchives and says the kid is back', async () => {
  const l = log();
  assert.equal(await release(kid, { unarchive: async (id) => void l.push(`unarchive:${id}`), done: (t) => l.push(t), failed: () => {} }), true);
  assert.deepEqual(l.calls, ['unarchive:k1', 'Fix the flaky test is back at a desk']);
});
