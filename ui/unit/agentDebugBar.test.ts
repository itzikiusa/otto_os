import { test } from 'node:test';
import assert from 'node:assert/strict';
import { loadSource } from './sourceHarness.ts';
import { debugBarView } from '../src/lib/uiCommands/agentDebugBar.ts';

// The driving bar while an agent drives a debug session (D-012): it stays past
// the one-minute linger, keeps the agent's narration while paused, offers
// Proceed once per stop (PTY: typed in; Threads: queued), and Stop hands the
// session to the person.

function store(sessions: { id: string; meta: Record<string, unknown> }[] = [{ id: 's1', meta: { ui_control: { enabled: true } } }]) {
  const posts: { path: string; body: unknown }[] = [];
  const ws = { sessions, otherWsSessions: [], panes: [], getSession: (id: string) => sessions.find((s) => s.id === id) ?? null, navigateToSession() {} };
  const { uiControl } = loadSource(new URL('../src/lib/stores/uiControl.svelte.ts', import.meta.url), {
    '../api/client': {
      api: {
        post: async (path: string, body: { enabled?: boolean }) => {
          posts.push({ path, body });
          return { id: 's1', meta: { ui_control: { enabled: body.enabled } } };
        },
      },
    },
    '../storage': { lsGet: () => null, lsSet() {} },
    '../toast.svelte': { toasts: { push() {}, info() {} } },
    '../toastError': { toastError() {} },
    './workspace.svelte': { ws },
    '../router.svelte': { router: { module: 'ide' } },
    '../desktop': { isEmbedded: false },
    '../sidebar': { moduleLabel: (m: string) => m },
    '../uiCommands/frames': loadSource(new URL('../src/lib/uiCommands/frames.ts', import.meta.url), {}),
    '../uiCommands/writeMemory': loadSource(new URL('../src/lib/uiCommands/writeMemory.ts', import.meta.url), {}),
  });
  uiControl.installAbort(() => {});
  return { uiControl, posts };
}

const plain = <T>(v: T): T => JSON.parse(JSON.stringify(v)) as T;
const claude = { session_id: 's1', title: 'Fix orders', provider: 'claude' };

test('the bar outlives the one-minute linger while the agent’s debuggee lives, and keeps its narration', () => {
  const { uiControl } = store();
  uiControl.begin('c1', claude, 'ide_debug_start', 'ide');
  uiControl.startDebug(claude, 'd1');
  uiControl.debugSay('s1', 'Stopped in CreateOrder: req.Items is empty');
  uiControl.end('c1', true);
  uiControl.debugState('d1', 3, false);
  const later = Date.now() + 10 * 60_000;
  assert.equal(uiControl.visible(later), true, 'not gone after a minute');
  const view = debugBarView(uiControl.driving, uiControl.agentDebug);
  assert.equal(view.say, 'Stopped in CreateOrder: req.Items is empty');
  assert.equal(view.proceed, true);
  // The session ends: back to the ordinary linger.
  uiControl.debugState('d1', null, true);
  assert.equal(uiControl.agentDebug, null);
  assert.equal(uiControl.visible(later), false);
});

test('Proceed shows only while paused and idle, sends once per stop (terminal session: typed in)', async () => {
  const { uiControl, posts } = store();
  uiControl.begin('c1', claude, 'ide_debug_start', 'ide');
  uiControl.startDebug(claude, 'd1');
  assert.equal(debugBarView(uiControl.driving, uiControl.agentDebug).proceed, false, 'running: no Proceed');
  uiControl.debugState('d1', 1, false);
  assert.equal(debugBarView(uiControl.driving, uiControl.agentDebug).proceed, false, 'a command still runs');
  uiControl.end('c1', true);
  assert.equal(debugBarView(uiControl.driving, uiControl.agentDebug).proceed, true);
  await uiControl.proceed();
  await uiControl.proceed();
  assert.deepEqual(plain(posts), [{ path: '/sessions/s1/message', body: { text: 'proceed' } }]);
  assert.equal(debugBarView(uiControl.driving, uiControl.agentDebug).proceed, false, 'sent for this stop');
  uiControl.debugState('d1', 2, false);
  assert.equal(debugBarView(uiControl.driving, uiControl.agentDebug).proceed, true, 'the next stop offers it again');
});

test('a Threads thread gets "proceed" queued as its next turn', async () => {
  const { uiControl, posts } = store();
  const thread = { ...claude, session_id: 'rt1', runtime: 'structured' };
  uiControl.begin('c1', thread, 'ide_debug_control', 'ide');
  uiControl.end('c1', true);
  uiControl.startDebug(thread, 'd1');
  uiControl.debugState('d1', 4, false);
  await uiControl.proceed();
  assert.deepEqual(plain(posts), [{ path: '/runtime-sessions/rt1/prompt', body: { text: 'proceed', delivery: 'queue' } }]);
});

test('Stop ends the agent’s control and leaves the debug session to the person', async () => {
  const { uiControl, posts } = store();
  uiControl.begin('c1', claude, 'ide_debug_start', 'ide');
  uiControl.end('c1', true);
  uiControl.startDebug(claude, 'd1');
  uiControl.debugState('d1', 1, false);
  await uiControl.stop();
  assert.equal(uiControl.agentDebug?.released, true, 'the IDE shows “stopped driving”');
  assert.equal(debugBarView(uiControl.driving, uiControl.agentDebug).proceed, false);
  assert.deepEqual(plain(posts.map((p) => p.path)), ['/sessions/s1/ui-control'], 'no stop command: the session stays live');
  // The grant flipped elsewhere / the session went away: released too.
  uiControl.startDebug(claude, 'd2');
  uiControl.applyEvent({ type: 'session_removed', session_id: 's1' });
  assert.equal(uiControl.agentDebug?.released, true);
});
