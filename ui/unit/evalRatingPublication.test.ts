import { test } from 'node:test';
import assert from 'node:assert/strict';
import { componentFunctions } from './componentFunctions.ts';
import { deferred } from './sourceHarness.ts';

const component = new URL('../src/modules/skills-eval/RunDetail.svelte', import.meta.url);
function fixture(api: Record<string, unknown>, extra: Record<string, unknown> = {}) {
  return componentFunctions(component, ['rate', 'poll'], {
    run: { id: 'run', human: 'old' }, evalId: 'run', disposed: false,
    seq: { gen: 1 }, rating: new Set(), scoreRevision: 0, pollCount: 0, lastPolled: '',
    skillsEvalApi: api, onupdate() {}, toastError() {}, load: async () => {},
    isActive: () => false, schedulePoll() {}, ...extra,
  });
}

test('failed rating refreshes the durable pending state before allowing retry', async () => {
  const refreshed: string[] = [];
  const state = fixture({ rate: async () => { throw new Error('publication failed'); } }, {
    load: async (id: string) => {
      refreshed.push(id);
      state.run = { id, human: 'new', scoring: { proof_status: 'pending' } };
    },
  });
  await state.rate({ id: 'iteration' }, 1);
  assert.deepEqual(refreshed, ['run']);
  assert.equal(state.run.scoring.proof_status, 'pending');
  assert.equal(state.rating.size, 0);
});

test('rating response after disposal cannot publish or refresh the old view', async () => {
  for (const failure of [false, true]) {
    const response = deferred<unknown>();
    let published = 0, refreshed = 0, errors = 0;
    const state = fixture({ rate: () => response.promise }, {
      onupdate() { published++; }, load: async () => { refreshed++; },
      toastError() { errors++; },
    });
    const pending = state.rate({ id: 'iteration' }, 1);
    state.disposed = true;
    if (failure) response.reject(new Error('late failure'));
    else response.resolve({ id: 'run', human: 'late' });
    await pending;
    assert.equal(published + refreshed + errors, 0);
  }
});

test('a poll started before a rating response cannot restore its stale score', async () => {
  const response = deferred<unknown>();
  const state = fixture({
    get: () => response.promise,
    rate: async () => ({ id: 'run', human: 'new' }),
  });
  state.load = async () => { state.run = { id: 'run', human: 'new' }; };
  const pending = state.poll();
  await state.rate({ id: 'iteration' }, 1);
  response.resolve({ id: 'run', human: 'old' });
  await pending;
  assert.equal(state.run.human, 'new');
});

test('a current successful rating reloads the latest report rather than its response snapshot', async () => {
  const updates: unknown[] = [];
  const result = { id: 'run', human: 'new' };
  const state = fixture({ rate: async () => ({ id: 'run', human: 'response snapshot' }) }, {
    onupdate(value: unknown) { updates.push(value); },
  });
  state.load = async () => { state.run = result; state.onupdate(result); };
  await state.rate({ id: 'iteration' }, 1);
  assert.equal(state.run, result);
  assert.deepEqual(updates, [result]);
});

test('out-of-order rating replies still reload after the final write completes', async () => {
  const first = deferred<unknown>(), second = deferred<unknown>();
  const state = fixture({ rate: (_run: string, it: string) => it === 'first' ? first.promise : second.promise });
  let durable = 'old';
  state.load = async () => { state.seq.gen++; state.run = { id: 'run', human: durable }; };
  const one = state.rate({ id: 'first' }, 1);
  const two = state.rate({ id: 'second' }, 2);
  durable = 'second complete'; second.resolve({ id: 'run', human: durable }); await two;
  durable = 'both complete'; first.resolve({ id: 'run', human: 'earlier response snapshot' }); await one;
  assert.equal(state.run.human, 'both complete');
});
