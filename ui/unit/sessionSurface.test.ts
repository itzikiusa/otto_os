import { test } from 'node:test';
import assert from 'node:assert/strict';
import { loadSource, deferred } from './sourceHarness.ts';

function setup() {
  const saved = deferred<void>();
  const calls: unknown[] = [];
  const subject = loadSource(new URL('../src/lib/sessionSurface.ts', import.meta.url), {
    './stores/workspace.svelte': { ws: { myRole: 'owner', updateSessionMeta: (id: string, meta: unknown) => { calls.push([id, meta]); return saved.promise; } } },
    './router.svelte': { router: { go: (route: string) => calls.push(route) } },
    './toastError': { toastError: (title: string) => calls.push(title) },
    './stores/sessionBuckets': { isForeground: (s: { meta: Record<string, unknown> }) => !s.meta.source },
  });
  return { openSessionSurface: subject.openSessionSurface, canSwitchSessionSurface: subject.canSwitchSessionSurface, saved, calls };
}
const agent = (meta: Record<string, unknown> = {}) => ({ id: 'same-id', kind: 'agent', provider: 'claude', meta, archived: false });

test('view handoff preserves session identity and only navigates after the surface patch succeeds', async () => {
  const s = setup();
  const input = agent({ provider_session_id: 'native-id', delegated_by: 'parent', permission_mode: 'default' });
  const pending = s.openSessionSurface(input, 'threads');
  assert.equal(JSON.stringify(s.calls), JSON.stringify([['same-id', { surface: 'threads' }]]));
  s.saved.resolve();
  await pending;
  assert.equal(s.calls[1], 'threads/same-id');
  assert.equal(input.meta.delegated_by, 'parent');
});

test('failed handoff stays on the current view and gives a recoverable error', async () => {
  const s = setup();
  const pending = s.openSessionSurface(agent(), 'threads');
  s.saved.reject(new Error('offline'));
  assert.equal(await pending, false);
  assert.equal(s.calls.length, 2);
  assert.match(String(s.calls[1]), /Couldn’t open/);
});

test('return to terminal uses the same session and never closes or spawns one', async () => {
  const s = setup();
  s.saved.resolve();
  await s.openSessionSurface(agent({ surface: 'threads' }), 'agents');
  assert.equal(JSON.stringify(s.calls), JSON.stringify([['same-id', { surface: 'agents' }], 'agents/same-id']));
});

test('runtime sessions, plain shells, background agents and embedded children cannot be moved', () => {
  const s = setup();
  for (const input of [agent({ runtime: 'structured' }), agent({ side_of: 'parent' }), agent({ threads_terminal: true }), agent({ source: 'assistant' }), { ...agent(), provider: 'shell' }, { ...agent(), kind: 'connection' }, { ...agent(), archived: true }]) {
    assert.equal(s.canSwitchSessionSurface(input), false);
  }
  assert.equal(s.canSwitchSessionSurface({ ...agent({ nested_provider: 'claude' }), provider: 'shell' }), true);
  assert.equal(s.canSwitchSessionSurface(agent({ delegated_by: 'parent' })), true);
});
