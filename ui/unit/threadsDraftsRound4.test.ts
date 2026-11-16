// Round 4 (review K): Threads composer attachment budget and draft refusals
// (K1), draft roots released with their view (K2), one acknowledgement per
// send (K8), the scrolled-up render window (K5), live output across a snapshot
// (K6) and the "no git account" PR state (K4).
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { loadSource } from './sourceHarness.ts';
import {
  DraftPersistence,
  MAX_DRAFT_JSON_BYTES,
  acknowledgeComposer,
  emptyDraft,
  snapshotDraft,
} from '../src/lib/stores/runtimeDraftPersistence.ts';
import { renderWindow, timelineRowKey } from '../src/modules/threads/runtime/follow.ts';
import { applyEvents, applyReplay, emptyProjection } from '../src/modules/threads/runtime/projection.ts';
import type { GitAccount, Repo, RuntimeDraftContent, RuntimeEvent, RuntimeEventBody, TurnItem } from '../src/lib/api/types';

const MiB = 1024 * 1024;

function loadComposer(toasts: string[] = []) {
  return loadSource(
    new URL('../src/modules/threads/runtime/runtimeComposer.ts', import.meta.url),
    {
      '../../../lib/toastError': { toastError: (title: string, e: unknown) => toasts.push(`${title}: ${String(e)}`) },
      '../../../lib/telemetry': { startMeasurement: () => null },
      './hostCommands': { runHostSlash: () => false },
      './timeline': { errorText: (s: string) => s },
      '../../../lib/stores/runtimeDrafts.svelte': { runtimeDrafts: { owns: () => true } },
    },
    { btoa },
  ) as typeof import('../src/modules/threads/runtime/runtimeComposer.ts');
}

/** A File stand-in: `size` is what the budget reads; the bytes stay tiny. */
const file = (name: string, size: number) =>
  ({ name, size, type: 'image/png', arrayBuffer: async () => new Uint8Array([1, 2, 3]).buffer }) as unknown as File;
/** An attachment already in the composer, `bytes` decoded. */
const attached = (name: string, bytes: number) => ({ name, mime: 'image/png', data_base64: 'A'.repeat(Math.ceil(bytes / 3) * 4) });

test('K1: the composer refuses the attachment that would pass the 16 MB message total, inline, keeping the rest', async () => {
  const toasts: string[] = [];
  const c = loadComposer(toasts);
  assert.equal(c.MAX_TOTAL_ATTACHMENT_BYTES, 16 * MiB);
  // Base64 of the whole budget plus 1 MiB of text stays under the daemon's 24 MiB draft ceiling.
  assert.ok((c.MAX_TOTAL_ATTACHMENT_BYTES * 4) / 3 + MiB < 24 * MiB);
  assert.equal(c.attachmentBytes([attached('a.png', 3 * MiB)]), 3 * MiB);

  const draft = { images: [attached('one.png', 8 * MiB), attached('two.png', 7 * MiB)], attachError: null as string | null };
  await c.appendDraftAttachments('A', draft, [file('three.png', 2 * MiB), file('small.png', 100)], () => true);
  assert.deepEqual([...draft.images.map((f) => f.name)], ['one.png', 'two.png', 'small.png'], 'the one that fits still joins');
  assert.match(draft.attachError ?? '', /three\.png/);
  assert.match(draft.attachError ?? '', /16 MB/);
  assert.deepEqual(toasts, [], 'shown under the composer, not as a toast');

  // Per-file cap names its own limit; nothing over it is read.
  const solo = { images: [] as ReturnType<typeof attached>[], attachError: null as string | null };
  await c.appendDraftAttachments('A', solo, [file('huge.png', 11 * MiB)], () => true);
  assert.equal(solo.images.length, 0);
  assert.match(solo.attachError ?? '', /huge\.png is over the 10 MB limit/);

  // A successful add clears an earlier refusal.
  await c.appendDraftAttachments('A', solo, [file('ok.png', 10)], () => true);
  assert.equal(solo.attachError, null);

  // Without an inline target (stage / IDE), a batch is still budgeted and refused by toast.
  const read = await c.readAttachments([file('x.png', 9 * MiB), file('y.png', 9 * MiB)], () => true);
  assert.equal(read.length, 1);
  assert.equal(toasts.length, 1);
  assert.match(toasts[0], /y\.png/);
});

test('K1: queued-edit restore never pushes the composer past the byte budget', () => {
  const toasts: string[] = [];
  const c = loadComposer(toasts);
  const merged = c.mergeQueuedAttachments([attached('kept.png', 12 * MiB)], [attached('big.png', 6 * MiB), attached('fits.png', MiB)]);
  assert.deepEqual([...merged.map((f) => f.name)], ['kept.png', 'fits.png']);
  assert.match(toasts[0] ?? '', /16 MB/);
});

test('K1: a draft the daemon refuses for size says why and is not re-uploaded until it shrinks', async () => {
  const draft = emptyDraft();
  const statuses: [string, string | undefined][] = [];
  let puts = 0;
  const tooLarge = Object.assign(new Error('Draft exceeds 24 MiB. Remove attachments and retry.'), { status: 413 });
  const p = new DraftPersistence(
    draft,
    {
      load: async () => ({ revision: 0, draft: null }),
      save: async (d: RuntimeDraftContent, r: number) => {
        puts++;
        if (d.images.length > 1) throw tooLarge;
        return { revision: r + 1, draft: d };
      },
    } as never,
    () => true,
    (s, m) => statuses.push([s, m]),
  );
  await p.load();
  draft.images = [attached('a.png', 30), attached('b.png', 30)];
  await p.save();
  assert.equal(puts, 1);
  assert.deepEqual(statuses.at(-1), ['error', 'Draft exceeds 24 MiB. Remove attachments and retry.']);
  assert.ok(!statuses.some(([, m]) => /Retry when connected/.test(m ?? '')));
  // Typing on (each pause in typing saves) does not upload the same oversized draft again.
  draft.text = 'more words';
  await p.save();
  draft.text = 'more words still';
  await p.save();
  assert.equal(puts, 1);
  assert.equal(statuses.at(-1)?.[0], 'error', 'the error stays shown');
  // Removing an attachment shrinks it: saved.
  draft.images = [draft.images[0]];
  await p.save();
  assert.equal(puts, 2);
  assert.equal(statuses.at(-1)?.[0], 'saved');
});

test('K1: a draft over the 24 MiB ceiling is never sent', async () => {
  const draft = emptyDraft();
  let puts = 0;
  let last: string | undefined;
  const p = new DraftPersistence(
    draft,
    { load: async () => ({ revision: 0, draft: null }), save: async (d: never) => { puts++; return { revision: 1, draft: d }; } } as never,
    () => true,
    (_s, m) => (last = m),
  );
  await p.load();
  draft.images = [{ name: 'x.png', mime: 'image/png', data_base64: 'A'.repeat(MAX_DRAFT_JSON_BYTES) }];
  await p.save();
  assert.equal(puts, 0);
  assert.match(last ?? '', /24 MB/);
});

test('K8: one acknowledgement per send keeps an identical image pasted while it was in flight', () => {
  const image = { name: 'shot.png', mime: 'image/png', data_base64: 'AA==' };
  const composer = { text: 'hi', images: [{ ...image }], context: [], model: undefined, effort: null, pickedMode: null, freshContext: false };
  const submitted = snapshotDraft(composer);
  acknowledgeComposer(composer, submitted); // at submit
  assert.equal(composer.images.length, 0);
  composer.images = [{ ...image }]; // pasted again while the request runs
  acknowledgeComposer(composer, submitted); // after the daemon answered
  assert.equal(composer.images.length, 1);
  assert.equal(composer.text, '');
});

test('K2: a draft syncs only while a view holds it, and idle drafts past the LRU are saved then dropped', async () => {
  const { importRunes } = await import('./runesHarness.ts');
  const mod = await importRunes<typeof import('../src/lib/stores/runtimeDrafts.svelte.ts')>(
    new URL('../src/lib/stores/runtimeDrafts.svelte.ts', import.meta.url),
  );
  const store = new mod.RuntimeDrafts();
  const live = new Set<string>();
  const disposed: [string, boolean][] = [];
  const afters = new Map<string, Promise<void> | undefined>();
  let finishSave!: () => void;
  store.configure((id, _draft, after) => {
    afters.set(id, after);
    return {
      activate: () => {
        live.add(id);
        return () => live.delete(id);
      },
      dispose: (flush) => {
        disposed.push([id, !!flush]);
        return flush && id === 'T0' ? new Promise<void>((r) => (finishSave = r)) : undefined;
      },
    };
  });
  // Opening a thread: the view holds its draft, which starts the live sync.
  const t0 = store.for('T0');
  const release0 = store.hold('T0');
  assert.deepEqual([...live], ['T0']);
  assert.equal(store.held('T0'), true);
  release0();
  assert.deepEqual([...live], [], 'leaving the thread stops its focus/turn-event GETs');
  assert.equal(store.for('T0'), t0, 'a recent draft stays in memory');

  // A draft that failed to save is never dropped (its words may exist nowhere else).
  store.for('ERR').saveStatus = 'error';
  for (let i = 1; i < mod.IDLE_DRAFTS_KEPT; i++) {
    const release = store.hold(`T${i}`);
    store.for(`T${i}`).text = `draft ${i}`;
    release();
  }
  assert.deepEqual(disposed, [['T0', true]], 'the oldest idle draft is saved, then dropped');
  assert.equal(store.owns('T0', t0), false);
  assert.ok(store.owns('ERR', store.for('ERR')));

  // Back on T0 while its last save still runs: the new root reads after it lands.
  store.for('T0');
  const after = afters.get('T0');
  assert.ok(after, 'reload waits for the evicted save');
  let landed = false;
  void after!.then(() => (landed = true));
  await Promise.resolve();
  assert.equal(landed, false);
  finishSave();
  await new Promise((r) => setTimeout(r, 0));
  assert.equal(landed, true);

  // An identity change hard-stops everything.
  store.setIdentity('other');
  assert.ok(disposed.some(([id, flush]) => id === 'ERR' && !flush));
});

test('K5: while the reader is scrolled up, a new row never drops a row off the top', () => {
  const rows = Array.from({ length: 205 }, (_, i) => ({ kind: 'work', id: `w${i}` }));
  const key = timelineRowKey;
  assert.equal(key({ kind: 'message', item: { id: 'm1' } }), 'm1');
  // Pinned: the newest 200.
  const pinned = renderWindow(rows, 200, null, key);
  assert.equal(pinned.length, 200);
  assert.equal(key(pinned[0]), 'w5');
  // Scrolled up with w5 first on screen; three rows arrive.
  const grown = [...rows, ...Array.from({ length: 3 }, (_, i) => ({ kind: 'work', id: `n${i}` }))];
  const held = renderWindow(grown, 200, 'w5', key);
  assert.equal(key(held[0]), 'w5', 'the window still starts where the reader left it');
  assert.equal(held.length, 203);
  // Load earlier (a larger cap) still shows older rows; an unknown anchor falls back to the cap.
  assert.equal(key(renderWindow(grown, 300, 'w5', key)[0]), 'w0');
  assert.equal(key(renderWindow(grown, 200, 'gone', key)[0]), 'w8');
});

test('K6: a snapshot reload keeps the live output of a command that is still running', () => {
  const TS = '2026-11-16T10:00:00Z';
  const ev = (sequence: number, body: RuntimeEventBody): RuntimeEvent => ({ session_id: 's1', sequence, ts: TS, ...body }) as RuntimeEvent;
  const cmd = (id: string, status: 'running' | 'completed', ordinal: number): TurnItem =>
    ({ id, turn_id: 't1', ordinal, status, title: null, started_at: TS, completed_at: null, updated_at: TS, type: 'command_execution', input: 'cargo test' }) as TurnItem;
  const { projection: p } = applyEvents(emptyProjection(), [
    ev(1, { type: 'turn_item.updated', turn_item: cmd('run', 'running', 1) }),
    ev(2, { type: 'turn_item.updated', turn_item: cmd('done', 'running', 2) }),
    ev(3, { type: 'command_output.delta', item_id: 'run', chunk: 'test a ... ok\n' }),
    ev(4, { type: 'command_output.delta', item_id: 'done', chunk: 'finished\n' }),
  ] as RuntimeEvent[]);
  const next = applyReplay(p, {
    kind: 'snapshot',
    snapshot_sequence: 50,
    snapshot: {
      provider_session: null,
      provider_thread: null,
      turns: [],
      turn_items: [cmd('run', 'running', 1), cmd('done', 'completed', 2)],
      runtime_requests: [],
      plans: [],
      queued_prompts: [],
    },
  } as never);
  assert.equal(next.output.run, 'test a ... ok\n');
  assert.equal('done' in next.output, false);
});

test('K4: a repo with no account for its forge is reported as "needs an account", not "no PR"', async () => {
  let accounts: GitAccount[] = [];
  const prs = loadSource(
    new URL('../src/modules/threads/threadPrs.svelte.ts', import.meta.url),
    {
      'lib/api/client': { api: { get: async () => accounts } },
      'lib/api/git': { gitApi: { prs: { list: async () => ({ items: [], has_more: false }) } } },
    },
    {},
    { relative: true },
  ).threadPrs as typeof import('../src/modules/threads/threadPrs.svelte.ts')['threadPrs'];
  const repo = {
    id: 'r1', workspace_id: 'w', name: 'demo', path: '/demo', remote_url: 'git@github.com:o/demo.git',
    provider: 'github', git_account_id: null, created_at: '2026-11-16T00:00:00Z',
  } as Repo;
  prs.prFor(repo, 'feat');
  await new Promise((r) => setTimeout(r, 15));
  assert.equal(prs.checked(repo, 'feat'), true);
  assert.equal(prs.needsAccount(repo, 'feat'), true);
  accounts = [{ id: 'a', provider: 'github', username: 'me' } as GitAccount];
  prs.forget(repo, 'feat');
  assert.equal(prs.needsAccount(repo, 'feat'), false);
  prs.prFor(repo, 'feat');
  await new Promise((r) => setTimeout(r, 15));
  assert.equal(prs.needsAccount(repo, 'feat'), false);
});
