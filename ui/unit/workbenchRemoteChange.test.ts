// S18-06: a remote change while this window is dirty must HOLD autosave
// (banner: Reload / Keep mine) instead of overwriting the other window's save;
// content saves carry `if_hash`, a 409 raises the banner, and only Keep mine
// saves unconditionally. Coalesced autosaves keep `rev`, so the event's
// `content_hash` decides staleness.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { loadSource } from './sourceHarness.ts';

class ApiError extends Error {
  status: number;
  code: string;
  constructor(status: number, message: string) {
    super(message);
    this.status = status;
    this.code = 'conflict';
  }
}

function setup(opts: { conflict?: boolean } = {}) {
  const patches: Array<Record<string, unknown>> = [];
  let liveHandler: ((ev: Record<string, unknown>) => void) | null = null;
  const doc = { id: 'd1', rev: 2, content_hash: 'h-base', name: 'a.sql', content: 'base' };
  const mod = loadSource(new URL('../src/modules/workbench/workbench.svelte.ts', import.meta.url), {
    svelte: { untrack: (fn: () => unknown) => fn() },
    '../../lib/api/workbench': {
      createWorkbenchDoc: async () => ({}),
      getWorkbenchDoc: async () => ({ ...doc }),
      listWorkbenchDocs: async () => [doc],
      purgeWorkbenchDoc: async () => {},
      restoreWorkbenchDoc: async () => ({}),
      trashWorkbenchDoc: async () => ({}),
      updateWorkbenchDoc: async (_ws: string, _id: string, body: Record<string, unknown>) => {
        patches.push(body);
        if (opts.conflict && body.if_hash) throw new ApiError(409, 'stale');
        return { ...doc, rev: 3, content_hash: `h-${String(body.content)}` };
      },
    },
    '../../lib/loadError': { loadErrorText: (e: unknown) => String(e) },
    '../../lib/api/client': { ApiError },
    '../../lib/live': {
      onLive: (_t: string[], cb: (ev: Record<string, unknown>) => void) => { liveHandler = cb; return () => {}; },
      appLive: { onResync: () => () => {} },
    },
  });
  const wb = mod.workbench;
  return { wb, patches, emit: (ev: Record<string, unknown>) => liveHandler?.(ev) };
}

const tick = () => new Promise((r) => setTimeout(r, 0));

async function opened(s: ReturnType<typeof setup>) {
  await s.wb.attach('w1');
  await s.wb.ensureLoaded('d1');
  await tick();
  assert.equal(s.wb.open.d1.buffer, 'base');
}

test('a remote change while dirty cancels the pending autosave and holds later ones', async () => {
  const s = setup();
  await opened(s);
  s.wb.setBuffer('d1', 'mine');
  // Window 2 autosaved (coalesced: same rev, new hash).
  s.emit({ type: 'workbench_doc_changed', workspace_id: 'w1', doc_id: 'd1', action: 'updated', rev: 2, content_hash: 'h-theirs' });
  assert.equal(s.wb.open.d1.remoteChanged, true);
  s.wb.setBuffer('d1', 'mine, more typing');
  await new Promise((r) => setTimeout(r, 900)); // > AUTOSAVE_MS
  await s.wb.save('d1', false); // a direct save is held too
  assert.equal(s.patches.length, 0, 'nothing overwrote the other window');
  // Keep mine: one unconditional save.
  s.wb.keepMine('d1');
  await tick();
  assert.equal(s.patches.length, 1);
  assert.equal(s.patches[0].content, 'mine, more typing');
  assert.equal(s.patches[0].if_hash, undefined);
});

test('content saves carry if_hash; a 409 raises the banner instead of an error', async () => {
  const s = setup({ conflict: true });
  await opened(s);
  s.wb.setBuffer('d1', 'mine');
  await s.wb.save('d1', false);
  assert.equal(s.patches[0].if_hash, 'h-base');
  assert.equal(s.wb.open.d1.remoteChanged, true);
  assert.equal(s.wb.open.d1.saveError, null);
  assert.equal(s.wb.isDirty('d1'), true, 'the buffer is kept');
});

test('an event with our own hash (or an older rev) is ignored', async () => {
  const s = setup();
  await opened(s);
  s.wb.setBuffer('d1', 'mine');
  s.emit({ type: 'workbench_doc_changed', workspace_id: 'w1', doc_id: 'd1', action: 'updated', rev: 2, content_hash: 'h-base' });
  s.emit({ type: 'workbench_doc_changed', workspace_id: 'w1', doc_id: 'd1', action: 'updated', rev: 1, content_hash: 'h-old' });
  assert.equal(s.wb.open.d1.remoteChanged, false);
});
