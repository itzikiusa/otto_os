// The ⌘K switcher's words come from the sidebar's own rules
// (threadSwitcherSource.svelte.ts): "Done" is thread_seen_at (threadMeta),
// not this window's in-memory flag — after a reload too.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { register } from 'node:module';
import { loadSource } from './sourceHarness.ts';

const hook =
  'export async function resolve(s, c, n) {' +
  "  if (/^\\.{1,2}\\//.test(s) && !/\\.[a-z]+$/i.test(s)) { try { return await n(s + '.ts', c); } catch {} }" +
  '  return n(s, c);' +
  '}';
register('data:text/javascript,' + encodeURIComponent(hook));
const inbox = await import('../src/modules/threads/threadInbox.ts');
const switcher = await import('../src/modules/threads/threadSwitcher.ts');

function rows(unreadById: Record<string, boolean>) {
  const row = { id: 'a', title: 'A', cwd: '/w/p', last_active_at: '2026-11-23T10:00:00Z', meta: {} };
  const { threadSwitcherSource } = loadSource(new URL('../src/modules/threads/threadSwitcherSource.svelte.ts', import.meta.url), {
    '../../lib/paletteThreads.svelte': { paletteThreads: { install: () => () => {} } },
    '../../lib/stores/now.svelte': { now: () => 0, rel: () => '1h' },
    '../../lib/stores/workspace.svelte': { ws: { sessions: [], unread: {}, currentId: null } },
    './runtime/runtimeThreads.svelte': { isRuntimeThread: () => true, runtimeThreads: { rows: () => [row], unread: {} } },
    './threadInbox': inbox,
    './threadInbox.svelte': { inboxLive: { kindsOf: () => [], since: {}, ensure: () => {}, mount: () => () => {} } },
    './threadOrigins': { threadOrigin: () => 'main' },
    './threadNav.svelte': { threadNav: { order: ['a'] }, threadRowState: () => ({ key: 'idle', label: 'Idle' }) },
    './threadSwitcher': switcher,
    './threadsHelpers': { isThreadSession: () => true, projectNameFromCwd: () => 'p' },
    './threadMeta.svelte': { threadMeta: { unread: (id: string) => unreadById[id] === true } },
    './threadsStore.svelte': { threads: { focusedSessionId: null, newDraftMode: false } },
  }) as { threadSwitcherSource: { rows: () => { id: string; word: string }[] } };
  return threadSwitcherSource.rows();
}

test('"Done" follows thread_seen_at, like the sidebar', () => {
  assert.equal(rows({ a: true })[0].word, 'Done');
  assert.equal(rows({ a: false })[0].word, '1h');
});
