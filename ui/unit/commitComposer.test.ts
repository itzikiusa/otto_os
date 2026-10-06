// The WIP commit composer survives navigation: typed text per repo, and an
// agent draft that lands while no panel shows the repo is parked, not lost.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { loadSource } from './sourceHarness.ts';

function composer() {
  const store = new Map<string, string>();
  const sessionStorage = {
    getItem: (k: string) => store.get(k) ?? null,
    setItem: (k: string, v: string) => void store.set(k, v),
    removeItem: (k: string) => void store.delete(k),
  };
  return loadSource(new URL('../src/modules/git/commitComposer.ts', import.meta.url), {}, { sessionStorage });
}

test('typed text is kept per repo and cleared when emptied', () => {
  const c = composer();
  c.writeComposer('r1', { subject: 'fix: x', body: 'why', draftedAt: null });
  assert.equal(c.readComposer('r1').subject, 'fix: x');
  assert.equal(c.readComposer('r2').subject, '', 'other repos untouched');
  c.writeComposer('r1', { subject: '', body: '', draftedAt: null });
  assert.equal(c.readComposer('r1').body, '');
});

test('a draft that lands with no panel watching is parked once', async () => {
  const c = composer();
  const reply = { message: 'feat: y', from_staged: true, session_id: 's' };
  const p = c.startDraft('r1', async () => reply);
  assert.equal(c.draftInFlight('r1'), p, 'a remounted panel can resume it');
  await p;
  await new Promise((r) => setImmediate(r));
  assert.equal(c.takePendingDraft('r1')?.message, 'feat: y');
  assert.equal(c.takePendingDraft('r1'), null, 'handed out once');
  assert.equal(c.draftInFlight('r1'), null);
});

test('a watched repo gets the reply directly (nothing parked)', async () => {
  const c = composer();
  const stop = c.watchComposer('r1');
  await c.startDraft('r1', async () => ({ message: 'm', from_staged: false }));
  await new Promise((r) => setImmediate(r));
  assert.equal(c.takePendingDraft('r1'), null);
  stop();
});
