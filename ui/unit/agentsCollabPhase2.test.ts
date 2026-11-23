import { test } from 'node:test';
import assert from 'node:assert/strict';
import type { Turn } from '../src/lib/api/types.ts';
import {
  buildRecapActionDraft,
  queueRecapActionThread,
  THREAD_DRAFT_HANDOFF_KEY,
} from '../src/modules/rooms/recapThreadHandoff.ts';
import { consumeThreadDraftHandoff } from '../src/modules/threads/recapThreadConsumer.ts';
import {
  buildRelayWithOutputDraft,
  latestTranscriptAnswer,
  summarizeRelayContext,
} from '../src/modules/threads/relayContext.ts';
import { componentFunctions } from './componentFunctions.ts';
import { deferred } from './sourceHarness.ts';

function memoryStorage() {
  const map = new Map<string, string>();
  return {
    getItem: (k: string) => map.get(k) ?? null,
    setItem: (k: string, v: string) => { map.set(k, v); },
    removeItem: (k: string) => { map.delete(k); },
  };
}

test('Room Recap action handoff formats draft, persists one-shot payload, and expires after 5 minutes', () => {
  const store = memoryStorage();
  const draft = queueRecapActionThread(
    {
      sessionTitle: 'Checkout latency triage',
      action: 'Add composite index on orders(tenant_id, created_at)',
      overview: 'Investigated p99 spike on checkout queries.',
      workspaceId: 'ws-1',
    },
    store,
  );
  assert.ok(draft.includes('Checkout latency triage'));
  assert.ok(draft.includes('> Add composite index on orders(tenant_id, created_at)'));
  assert.ok(draft.includes('Room recap context: Investigated p99 spike'));

  const consumed = consumeThreadDraftHandoff(store);
  assert.ok(consumed);
  assert.equal(consumed.workspaceId, 'ws-1');
  assert.equal(consumed.draft, draft);
  assert.equal(consumeThreadDraftHandoff(store), null, 'handoff must be consumed once');

  // Expired handoff (>5 min old) is discarded cleanly
  store.setItem(
    THREAD_DRAFT_HANDOFF_KEY,
    JSON.stringify({ draft: 'stale', workspaceId: 'ws-1', ts: Date.now() - 10 * 60_000 }),
  );
  assert.equal(consumeThreadDraftHandoff(store), null);

  // Overview clipping
  const longDraft = buildRecapActionDraft({
    sessionTitle: 'Room',
    action: 'Ship fix',
    overview: 'x'.repeat(400),
  });
  assert.ok(longDraft.includes('…'));
});

test('Thread relay context extracts latest assistant answer, strips ANSI, and builds quoted draft', () => {
  const turns: Turn[] = [
    {
      id: 't1',
      role: 'user',
      ts: null,
      blocks: [{ kind: 'text', md: 'Check migration 14' }],
      duration_ms: null,
      model: null,
      system: [],
      reasoning_steps: 0,
    },
    {
      id: 't2',
      role: 'assistant',
      ts: null,
      blocks: [{ kind: 'text', md: 'First reply' }],
      duration_ms: null,
      model: null,
      system: [],
      reasoning_steps: 0,
    },
    {
      id: 't3',
      role: 'assistant',
      ts: null,
      blocks: [
        { kind: 'thinking', count: 1 },
        { kind: 'text', md: '\x1b[32mMigration 14\x1b[0m adds a nullable column safely.' },
      ],
      duration_ms: null,
      model: null,
      system: [],
      reasoning_steps: 0,
    },
  ];
  const latest = latestTranscriptAnswer(turns);
  assert.equal(latest, 'Migration 14 adds a nullable column safely.');

  const summary = summarizeRelayContext(latest);
  assert.ok(summary);
  assert.equal(summary.preview, 'Migration 14 adds a nullable column safely.');
  assert.equal(summarizeRelayContext('   \n  '), null);

  const longSummary = summarizeRelayContext('a'.repeat(500), 100);
  assert.ok(longSummary);
  assert.ok(longSummary.snippet.length <= 100);
  assert.ok(longSummary.preview.length <= 84);

  const draft = buildRelayWithOutputDraft('DB Audit', 'Line 1\nLine 2');
  assert.equal(draft, 'Context from “DB Audit”:\n> Line 1\n> Line 2\n\n');
});

test('SyncProvidersModal loads diffs with stale-sequence guard and syncs library copy via putFile', async () => {
  const slowLib = deferred<{ content: string }>();
  const fastLib = deferred<{ content: string }>();
  let getCalls = 0;
  let putPayload: unknown = null;
  let synced = 0;
  let closed = 0;
  let keptCalled = 0;

  const state = componentFunctions(
    new URL('../src/modules/skills-lab/SyncProvidersModal.svelte', import.meta.url),
    ['loadDiffs', 'syncAll'],
    {
      name: 'go-review',
      providers: ['claude', 'codex'],
      activeProvider: '',
      libraryBody: null as string | null,
      providerBodies: {} as Record<string, string>,
      loading: false,
      loadError: null as string | null,
      syncing: false,
      seq: 0,
      disposed: false,
      skillLabApi: {
        getFile: () => (++getCalls === 1 ? slowLib.promise : fastLib.promise),
        getProvider: async (p: string) => ({ body: `# ${p} old copy` }),
        putFile: async (_name: string, body: unknown) => {
          putPayload = body;
          return { files: [], kept_copies: [{ provider: 'claude', backup_path: '/tmp/bak' }] };
        },
      },
      sourceLabel: (s: string) => s,
      plural: (n: number, w: string) => `${n} ${w}${n === 1 ? '' : 's'}`,
      loadErrorText: String,
      noteKeptCopies: () => { keptCalled++; },
      toasts: { success() {} },
      toastError() {},
      onsynced: () => { synced++; },
      onclose: () => { closed++; },
    },
  );

  // Stale load race: first resolves after second
  const p1 = state.loadDiffs();
  const p2 = state.loadDiffs();
  fastLib.resolve({ content: '# Fresh library copy' });
  await p2;
  slowLib.resolve({ content: '# Stale library copy' });
  await p1;
  assert.equal(state.libraryBody, '# Fresh library copy');
  assert.equal(state.activeProvider, 'claude');
  assert.equal(state.providerBodies.codex, '# codex old copy');

  await state.syncAll();
  assert.deepEqual(JSON.parse(JSON.stringify(putPayload)), {
    path: 'SKILL.md',
    content: '# Fresh library copy',
  });
  assert.equal(keptCalled, 1);
  assert.equal(synced, 1);
  assert.equal(closed, 1);
});

test('SkillEditor.quickEval saves dirty draft after confirm before calling onevaluate', async () => {
  let saved = 0;
  let evaluated = 0;
  const state = componentFunctions(
    new URL('../src/modules/skills-lab/SkillEditor.svelte', import.meta.url),
    ['quickEval'],
    {
      name: 'demo-skill',
      currentFile: 'SKILL.md',
      dirty: true,
      disposed: false,
      confirmer: { ask: async () => true },
      save: async () => { saved++; state.dirty = false; },
      onevaluate: () => { evaluated++; },
    },
  );
  await state.quickEval();
  assert.equal(saved, 1);
  assert.equal(evaluated, 1);
});
