import { test } from 'node:test';
import assert from 'node:assert/strict';
import { deferred, loadSource } from './sourceHarness.ts';
import type { ProviderTurn, RuntimeEvent, RuntimeReplay, TurnItem } from '../src/lib/api/types';

const flush = () => new Promise<void>((r) => setImmediate(r));
const plain = <T>(v: T): T => JSON.parse(JSON.stringify(v)) as T;

function msgEvent(sessionId: string, sequence: number, id: string, text: string, turnId = 't-1'): RuntimeEvent {
  const item: TurnItem = {
    id,
    turn_id: turnId,
    ordinal: sequence,
    type: 'assistant_message',
    message_id: id,
    status: 'completed',
    title: null,
    text,
    streaming: false,
    started_at: '2026-11-14T08:00:00Z',
    completed_at: '2026-11-14T08:00:01Z',
    updated_at: '2026-11-14T08:00:01Z',
  };
  return {
    session_id: sessionId,
    sequence,
    ts: '2026-11-14T08:00:01Z',
    type: 'turn_item.updated',
    turn_item: item,
  };
}

function makeRuntimeHarness(extraGlobals: Record<string, unknown> = {}) {
  const gets: Array<{ path: string; lane: 'fg' | 'bg'; gate: ReturnType<typeof deferred<RuntimeReplay>> }> = [];
  let liveListener: ((ev: { event: RuntimeEvent }) => void) | null = null;
  let resyncListener: (() => void) | null = null;
  let connListener: ((up: boolean) => void) | null = null;
  let currentLane: 'fg' | 'bg' = 'fg';

  const winListeners = new Map<string, Set<() => void>>();
  const docListeners = new Map<string, Set<() => void>>();
  const docState = {
    hidden: false,
    addEventListener(type: string, fn: () => void) {
      if (!docListeners.has(type)) docListeners.set(type, new Set());
      docListeners.get(type)!.add(fn);
    },
    removeEventListener(type: string, fn: () => void) {
      docListeners.get(type)?.delete(fn);
    },
  };
  const winState: {
    innerWidth?: number;
    addEventListener(type: string, fn: () => void): void;
    removeEventListener(type: string, fn: () => void): void;
  } = {
    addEventListener(type: string, fn: () => void) {
      if (!winListeners.has(type)) winListeners.set(type, new Set());
      winListeners.get(type)!.add(fn);
    },
    removeEventListener(type: string, fn: () => void) {
      winListeners.get(type)?.delete(fn);
    },
  };

  const timers = new Map<number, { fn: () => void; ms: number; interval: boolean }>();
  let nextTimerId = 1;

  const laneStub = {
    async inLane<T>(lane: 'bg' | 'int', fn: () => Promise<T>): Promise<T> {
      const prev = currentLane;
      currentLane = lane === 'bg' ? 'bg' : 'fg';
      try {
        return await fn();
      } finally {
        currentLane = prev;
      }
    },
    tagSignal() {},
  };

  const mod = loadSource(
    new URL('../src/modules/threads/runtime/runtimeView.svelte.ts', import.meta.url),
    {
      '../../../lib/api/client': {
        api: {
          get(path: string) {
            const gate = deferred<RuntimeReplay>();
            gets.push({ path, lane: currentLane, gate });
            return gate.promise;
          },
        },
      },
      '../../../lib/api/lane': laneStub,
      'lib/api/lane': laneStub,
      '../../../lib/telemetry': { startMeasurement: () => null },
      '../../../lib/live': {
        onLive(_types: string[], fn: (ev: { event: RuntimeEvent }) => void) {
          liveListener = fn;
          return () => {
            if (liveListener === fn) liveListener = null;
          };
        },
        appLive: {
          onResync(fn: () => void) {
            resyncListener = fn;
            return () => {
              if (resyncListener === fn) resyncListener = null;
            };
          },
          onConnection(fn: (up: boolean) => void) {
            connListener = fn;
            return () => {
              if (connListener === fn) connListener = null;
            };
          },
        },
      },
      'modules/threads/runtime/runtimeThreads.svelte': {
        runtimeThreads: {
          isActiveOrWaiting: () => false,
          onSequence: () => () => {},
        },
      },
    },
    {
      document: docState,
      window: winState,
      setTimeout(fn: () => void, ms: number) {
        const id = nextTimerId++;
        timers.set(id, {
          fn: () => {
            timers.delete(id);
            fn();
          },
          ms,
          interval: false,
        });
        return id;
      },
      clearTimeout(id: number) {
        timers.delete(id);
      },
      setInterval(fn: () => void, ms: number) {
        const id = nextTimerId++;
        timers.set(id, { fn, ms, interval: true });
        return id;
      },
      clearInterval(id: number) {
        timers.delete(id);
      },
      ...extraGlobals,
    },
    { relative: true },
  );

  return {
    views: new mod.RuntimeViews() as {
      projections: Record<
        string,
        {
          lastSequence: number;
          turns: ProviderTurn[];
          items: TurnItem[];
          hasOlder: boolean;
          oldestTurnOrdinal: number | null;
          olderTurns: number;
        }
      >;
      loadError: Record<string, string>;
      staleError: Record<string, string>;
      olderError: Record<string, string>;
      itemError: Record<string, string>;
      loadingOlder: Record<string, boolean>;
      loadingItem: Record<string, boolean>;
      open(id: string): () => void;
      loadOlder(id: string, limitTurns?: number): Promise<boolean>;
      ensureItemDetail(id: string, itemId: string): Promise<TurnItem | null>;
      retry(id: string): void;
      noteSessionSequence(id: string, seq: number | null | undefined): void;
      setSessionActiveProbe(fn: (id: string) => boolean): void;
    },
    gets,
    timers,
    docState,
    winState,
    emitLive: (ev: RuntimeEvent) => liveListener?.({ event: ev }),
    emitResync: () => resyncListener?.(),
    emitConnection: (up: boolean) => connListener?.(up),
    emitWindow: (type: string) => winListeners.get(type)?.forEach((fn) => fn()),
    emitDoc: (type: string) => docListeners.get(type)?.forEach((fn) => fn()),
  };
}

test('RuntimeViews dirty-replay loop catches events missed during in-flight GET + reconnect', async () => {
  const h = makeRuntimeHarness();
  const close = h.views.open('s-1');

  assert.equal(h.gets.length, 1);
  assert.equal(h.gets[0].path, '/runtime-sessions/s-1/events?after_sequence=0&tail_turns=40&max_lines=10000&compact=true');
  assert.equal(h.gets[0].lane, 'fg');

  // While the first GET is in flight, WS reconnects (or initial onConnection fires):
  // previously this returned early and silently dropped events 4..5!
  h.emitResync();
  assert.equal(h.gets.length, 1, 'second replay coalesces via dirty flag while first is in flight');

  h.gets[0].gate.resolve({
    kind: 'events',
    events: [
      msgEvent('s-1', 1, 'm-1', 'first'),
      msgEvent('s-1', 2, 'm-2', 'second'),
      msgEvent('s-1', 3, 'm-3', 'third'),
    ],
    last_sequence: 3,
  });
  await flush();

  // Immediately fires follow-up bg replay from sequence 3!
  assert.equal(h.gets.length, 2);
  assert.equal(h.gets[1].path, '/runtime-sessions/s-1/events?after_sequence=3&tail_turns=40&max_lines=10000&compact=true');
  assert.equal(h.gets[1].lane, 'bg');

  h.gets[1].gate.resolve({
    kind: 'events',
    events: [msgEvent('s-1', 4, 'm-4', 'autonomous wakeup reply')],
    last_sequence: 4,
  });
  await flush();

  assert.equal(h.views.projections['s-1']?.lastSequence, 4);
  assert.deepEqual(
    plain(h.views.projections['s-1']?.items.map((i) => (i.type === 'assistant_message' ? i.text : ''))),
    ['first', 'second', 'third', 'autonomous wakeup reply'],
  );

  close();
  assert.equal(h.timers.size, 0, 'closing last view cleans up all timers');
});

test('RuntimeViews catches initial WS race, noteSessionSequence lag, and auto-retries load/stale errors', async () => {
  const h = makeRuntimeHarness();
  h.views.setSessionActiveProbe(() => true);
  const close = h.views.open('s-2');

  // 1. Initial load fails -> sets loadError and schedules 1500ms retry.
  h.gets[0].gate.reject(new Error('daemon restarting'));
  await flush();
  assert.match(h.views.loadError['s-2'] ?? '', /daemon restarting/);

  const retryEntry = [...h.timers.values()].find((t) => t.ms === 1_500);
  assert.ok(retryEntry, 'expected scheduled backoff retry timer');
  assert.equal(retryEntry.ms, 1_500);

  // 2. Retry fires and succeeds -> clears loadError automatically!
  retryEntry.fn();
  await flush();
  assert.equal(h.gets.length, 2);
  h.gets[1].gate.resolve({
    kind: 'events',
    events: [msgEvent('s-2', 1, 'm-1', 'recovered')],
    last_sequence: 1,
  });
  await flush();
  assert.equal(h.views.loadError['s-2'], undefined);
  assert.equal(h.views.projections['s-2']?.lastSequence, 1);

  // 3. Initial WS `onConnection(true)` catches events published before WS attached.
  h.emitConnection(true);
  await flush();
  assert.equal(h.gets.length, 3);
  assert.equal(h.gets[2].path, '/runtime-sessions/s-2/events?after_sequence=1&tail_turns=40&max_lines=10000&compact=true');
  h.gets[2].gate.reject(new Error('transient disconnect'));
  await flush();
  assert.match(h.views.staleError['s-2'] ?? '', /transient disconnect/);

  // 4. `noteSessionSequence('s-2', 3)` sees session row ahead of projection (1 < 3) -> replays and clears staleError!
  h.views.noteSessionSequence('s-2', 3);
  await flush();
  assert.equal(h.gets.length, 4);
  h.gets[3].gate.resolve({
    kind: 'events',
    events: [msgEvent('s-2', 2, 'm-2', 'step 2'), msgEvent('s-2', 3, 'm-3', 'step 3')],
    last_sequence: 3,
  });
  await flush();
  assert.equal(h.views.staleError['s-2'], undefined);
  assert.equal(h.views.projections['s-2']?.lastSequence, 3);

  // 5. Periodic 4s pollWhileVisible liveness tick catches silent WS drop while visible.
  const liveness = [...h.timers.values()].find((t) => t.ms === 4_000);
  assert.ok(liveness);
  assert.equal(liveness.ms, 4_000);
  liveness.fn();
  await flush();
  assert.equal(h.gets.length, 5);
  assert.equal(h.gets[4].path, '/runtime-sessions/s-2/events?after_sequence=3&tail_turns=40&max_lines=10000&compact=true');
  h.gets[4].gate.resolve({ kind: 'events', events: [], last_sequence: 3 });
  await flush();

  close();
  assert.equal(h.timers.size, 0, 'closing last view stops pollWhileVisible and clears retry timers');
});

test('follow.ts prevents false scroll-up unpinning on content shrink and compensates auto-fold 0px', () => {
  const follow = loadSource(new URL('../src/modules/threads/runtime/follow.ts', import.meta.url), {});

  // Content shrinking (e.g. work fold collapsing from 1800px to 1400px) clamps scrollTop downward: NOT user scroll-up!
  assert.equal(follow.didScrollUpByUser(800, 1100, 1400, 1800), false);
  // Real upward wheel/touch scroll with unchanged or growing scrollHeight IS user scroll-up.
  assert.equal(follow.didScrollUpByUser(950, 1100, 1800, 1800), true);
  assert.equal(follow.didScrollUpByUser(950, 1100, 1920, 1800), true);

  // Auto-fold scroll compensation:
  // 1) When following bottom, RuntimeFollow pins to end, so RuntimeWorkFold adds 0 delta.
  assert.equal(follow.foldScrollCompensation(true, -120, 220, 40, 600), 0);
  // 2) When scrolled up and a work block above/inside viewport shrinks from bottom=240 to bottom=52 (-188px),
  //    compensate scrollTop by -188px so messages below stay at the exact same pixel!
  assert.equal(follow.foldScrollCompensation(false, -80, 240, 52, 600), -188);
  // 3) When a work block is completely below the viewport (`beforeTop >= viewportHeight`), moving nothing on screen -> 0.
  assert.equal(follow.foldScrollCompensation(false, 640, 920, 680, 600), 0);
});

test('liveStatus.ts and timelineRows surface autonomous Background updates, live activity, and background tasks', () => {
  const live = loadSource(
    new URL('../src/modules/threads/runtime/liveStatus.ts', import.meta.url),
    {},
    {},
    { relative: true },
  );
  const timeline = loadSource(
    new URL('../src/modules/threads/runtime/timeline.ts', import.meta.url),
    {},
    {},
    { relative: true },
  );

  assert.equal(live.formatLiveElapsed(0), '0s');
  assert.equal(live.formatLiveElapsed(14.9), '14s');
  assert.equal(live.formatLiveElapsed(128), '2m 08s');
  assert.equal(live.formatLiveElapsed(3845), '1h 04m');

  const turns: ProviderTurn[] = [
    {
      id: 't-user',
      ordinal: 1,
      origin: 'user',
      status: 'completed',
      started_at: '2026-11-14T08:00:00Z',
      completed_at: '2026-11-14T08:00:02Z',
    },
    {
      id: 'auto-1',
      ordinal: 2,
      origin: 'agent',
      status: 'running',
      started_at: '2026-11-14T08:15:00Z',
      completed_at: null,
    },
  ];
  const items: TurnItem[] = [
    {
      id: 'u-1',
      turn_id: 't-user',
      ordinal: 1,
      type: 'user_message',
      status: 'completed',
      title: null,
      text: 'Monitor the deploy timer',
      attachments: [],
      started_at: '2026-11-14T08:00:00Z',
      completed_at: '2026-11-14T08:00:00Z',
      updated_at: '2026-11-14T08:00:00Z',
    },
    {
      id: 'a-1',
      turn_id: 't-user',
      ordinal: 2,
      type: 'assistant_message',
      message_id: 'a-1',
      status: 'completed',
      title: null,
      text: 'Timer scheduled.',
      streaming: false,
      started_at: '2026-11-14T08:00:01Z',
      completed_at: '2026-11-14T08:00:02Z',
      updated_at: '2026-11-14T08:00:02Z',
    },
    {
      id: 'cmd-auto',
      turn_id: 'auto-1',
      ordinal: 3,
      type: 'command_execution',
      status: 'running',
      title: null,
      input: 'kubectl rollout status deploy/api',
      started_at: '2026-11-14T08:15:01Z',
      completed_at: null,
      updated_at: '2026-11-14T08:15:01Z',
    },
    {
      id: 'a-auto',
      turn_id: 'auto-1',
      ordinal: 4,
      type: 'assistant_message',
      message_id: 'a-auto',
      status: 'completed',
      title: null,
      text: 'Rollout finished cleanly!',
      streaming: false,
      started_at: '2026-11-14T08:15:04Z',
      completed_at: '2026-11-14T08:15:05Z',
      updated_at: '2026-11-14T08:15:05Z',
    },
  ];

  const rows = timeline.timelineRows(items, turns);
  assert.deepEqual(
    plain(rows.map((r: { kind: string; label?: string }) => (r.kind === 'agent_turn' ? `${r.kind}:${r.label}` : r.kind))),
    ['message', 'message', 'agent_turn:Background update', 'work', 'message'],
  );

  const proj = {
    lastSequence: 4,
    session: null,
    thread: null,
    turns,
    items: items.slice(0, 3),
    requests: [],
    plans: [],
    queued: [],
    output: {},
  };
  assert.deepEqual(plain(live.deriveLiveFooter(proj, '14s', true, [])), {
    mode: 'working',
    prefix: 'Background wakeup',
    activity: 'Running kubectl rollout status deploy/api',
    elapsed: '14s',
  });

  assert.deepEqual(
    plain(live.deriveLiveFooter(proj, null, false, [{ label: 'Timer (30s)', kind: 'timer', childSessionId: null }, { label: 'Swarm board wait', kind: 'agent', childSessionId: null }])),
    {
      mode: 'waiting_bg',
      text: 'Waiting on 1 subagent and 1 timer',
      detail: 'Timer (30s), Swarm board wait',
    },
  );
});

test('providers, askAgents, and personaProviders support opencode & pi and strictly honor disabled_providers', () => {
  const authMock = {
    meta: {
      providers: ['agy', 'codex', 'opencode', 'pi', 'kilo', 'shell'],
      default_provider: 'opencode',
      model_flags: { codex: true, agy: true, opencode: true, pi: true, kilo: false },
      tools: [],
    },
  };
  const providers = loadSource(new URL('../src/lib/providers.ts', import.meta.url), {
    './stores/auth.svelte': { auth: authMock },
  });

  assert.equal(providers.harnessLabel('opencode'), 'OpenCode');
  assert.equal(providers.harnessLabel('pi'), 'Pi');
  assert.equal(providers.harnessLabel('gemini'), 'Gemini');
  assert.equal(providers.harnessLabel('grok'), 'Grok');
  assert.equal(providers.providerSubtitle('agy'), 'Antigravity CLI');
  assert.equal(providers.providerSubtitle('opencode'), 'OpenCode CLI');
  assert.equal(providers.providerSubtitle('pi'), 'Pi CLI');
  assert.equal(providers.providerSubtitle('kilo'), 'Custom provider');
  assert.equal(providers.providerSupportsModel('opencode'), true);
  assert.equal(providers.providerSupportsModel('pi'), true);

  // `claude` was disabled in Settings -> Providers: must NOT appear in ordered Thread harnesses!
  assert.deepEqual(
    JSON.parse(JSON.stringify(providers.orderThreadHarnesses(providers.agentProviders()))),
    ['codex', 'agy', 'opencode', 'pi', 'kilo'],
  );

  const ask = loadSource(new URL('../src/modules/threads/runtime/askAgents.ts', import.meta.url), {});
  const askable = ask.askableProviders(providers.agentProviders());
  assert.deepEqual(JSON.parse(JSON.stringify(askable)), ['codex', 'agy', 'opencode', 'pi']);
  const mentions = ask.mentionOptions('', () => [], (p: string) => providers.harnessLabel(p), 10, askable);
  assert.equal(mentions.some((m: { provider: string }) => m.provider === 'claude'), false);
  assert.equal(mentions.some((m: { provider: string }) => m.provider === 'opencode'), true);

  const persona = loadSource(new URL('../src/modules/personal-agents/personaProviders.ts', import.meta.url), {
    '../../lib/providers': { harnessLabel: providers.harnessLabel, providerFamily: providers.providerFamily },
  });
  // pi carries a persona (for one launch), so it stays the pick; a `preferred`
  // that is not enabled falls to the first enabled persona CLI (`codex`).
  assert.equal(persona.personaDefault('pi', ['codex', 'opencode', 'pi']), 'pi');
  assert.equal(persona.personaDefault('claude', ['codex', 'opencode', 'pi']), 'codex');
  assert.equal(persona.personaDefault('opencode', ['codex', 'opencode', 'pi']), 'opencode');
});

test('RuntimeViews tail_turns snapshot, loadOlder pagination, and reconnect snapshot preserve loaded older history', async () => {
  const h = makeRuntimeHarness();
  const close = h.views.open('s-paged');
  assert.equal(h.gets[0].path, '/runtime-sessions/s-paged/events?after_sequence=0&tail_turns=40&max_lines=10000&compact=true');

  const mkTurn = (ord: number): ProviderTurn => ({
    id: `t-${ord}`,
    ordinal: ord,
    status: 'completed',
    started_at: '2026-11-15T00:00:00Z',
    completed_at: '2026-11-15T00:00:01Z',
    origin: 'user',
  });
  const mkMsg = (ord: number, text: string): TurnItem => ({
    id: `a-${ord}`,
    turn_id: `t-${ord}`,
    ordinal: ord,
    status: 'completed',
    title: null,
    started_at: '2026-11-15T00:00:00Z',
    completed_at: '2026-11-15T00:00:01Z',
    updated_at: '2026-11-15T00:00:01Z',
    type: 'assistant_message',
    message_id: `m-${ord}`,
    text,
    streaming: false,
  });

  // Initial open resolves with newest 2 turns (3..4) and reports 2 older turns before ordinal 3.
  h.gets[0].gate.resolve({
    kind: 'snapshot',
    snapshot_sequence: 10,
    snapshot: {
      provider_session: null,
      provider_thread: null,
      turns: [mkTurn(3), mkTurn(4)],
      turn_items: [mkMsg(3, 'reply 3'), mkMsg(4, 'reply 4')],
      runtime_requests: [],
      plans: [],
      queued_prompts: [],
      has_older: true,
      oldest_turn_ordinal: 3,
      older_turns: 2,
    },
  });
  await flush();

  assert.equal(h.views.projections['s-paged']?.hasOlder, true);
  assert.equal(h.views.projections['s-paged']?.oldestTurnOrdinal, 3);
  assert.equal(h.views.projections['s-paged']?.olderTurns, 2);

  // User triggers loadOlder -> requests before_turn_ordinal=3&limit_turns=40&max_lines=10000 and prepends turns 1..2!
  const pLoad = h.views.loadOlder('s-paged');
  assert.equal(h.views.loadingOlder['s-paged'], true);
  assert.equal(h.gets[1].path, '/runtime-sessions/s-paged/events?before_turn_ordinal=3&limit_turns=40&max_lines=10000&compact=true');
  h.gets[1].gate.resolve({
    kind: 'snapshot',
    snapshot_sequence: 10,
    snapshot: {
      provider_session: null,
      provider_thread: null,
      turns: [mkTurn(1), mkTurn(2)],
      turn_items: [mkMsg(1, 'reply 1'), mkMsg(2, 'reply 2')],
      runtime_requests: [],
      plans: [],
      queued_prompts: [],
      has_older: false,
      oldest_turn_ordinal: 1,
      older_turns: 0,
    },
  });
  assert.equal(await pLoad, true);
  assert.equal(h.views.loadingOlder['s-paged'], undefined);
  assert.equal(h.views.projections['s-paged']?.hasOlder, false);
  assert.deepEqual(
    plain(h.views.projections['s-paged']?.turns.map((t) => t.ordinal)),
    [1, 2, 3, 4],
  );

  // Subsequent reconnect ring-miss returns tail snapshot (turns 4..5, oldest_turn_ordinal=4):
  // applyReplay MUST keep already-loaded older turns 1..3 intact while updating turn 4..5!
  h.emitResync();
  await flush();
  h.gets[2].gate.resolve({
    kind: 'snapshot',
    snapshot_sequence: 15,
    snapshot: {
      provider_session: null,
      provider_thread: null,
      turns: [mkTurn(4), mkTurn(5)],
      turn_items: [mkMsg(4, 'reply 4 updated'), mkMsg(5, 'reply 5')],
      runtime_requests: [],
      plans: [],
      queued_prompts: [],
      has_older: true,
      oldest_turn_ordinal: 4,
      older_turns: 3,
    },
  });
  await flush();

  assert.equal(h.views.projections['s-paged']?.lastSequence, 15);
  assert.equal(h.views.projections['s-paged']?.hasOlder, false);
  assert.equal(h.views.projections['s-paged']?.oldestTurnOrdinal, 1);
  assert.deepEqual(
    plain(h.views.projections['s-paged']?.items.map((i) => (i.type === 'assistant_message' ? i.text : ''))),
    ['reply 1', 'reply 2', 'reply 3', 'reply 4 updated', 'reply 5'],
  );

  close();
});

test('timelineRows preserves strict === row references for untouched rows across streaming deltas', () => {
  const timeline = loadSource(
    new URL('../src/modules/threads/runtime/timeline.ts', import.meta.url),
    {},
    {},
    { relative: true },
  );

  const turns: ProviderTurn[] = [
    {
      id: 't-1',
      ordinal: 1,
      status: 'completed',
      started_at: '2026-11-14T10:00:00Z',
      completed_at: '2026-11-14T10:00:05Z',
      origin: 'user',
    },
    {
      id: 't-2',
      ordinal: 2,
      status: 'running',
      started_at: '2026-11-15T10:00:00Z',
      completed_at: null,
      origin: 'agent',
    },
  ];
  const u1: TurnItem = {
    id: 'u-1',
    turn_id: 't-1',
    ordinal: 1,
    status: 'completed',
    title: null,
    started_at: '2026-11-14T10:00:00Z',
    completed_at: '2026-11-14T10:00:00Z',
    updated_at: '2026-11-14T10:00:00Z',
    type: 'user_message',
    text: 'Check build',
  };
  const cmd1: TurnItem = {
    id: 'c-1',
    turn_id: 't-1',
    ordinal: 2,
    status: 'completed',
    title: 'cargo check',
    started_at: '2026-11-14T10:00:01Z',
    completed_at: '2026-11-14T10:00:03Z',
    updated_at: '2026-11-14T10:00:03Z',
    type: 'command_execution',
    input: 'cargo check',
    output: 'Finished',
    exit_code: 0,
  };
  const a1: TurnItem = {
    id: 'a-1',
    turn_id: 't-1',
    ordinal: 3,
    status: 'completed',
    title: null,
    started_at: '2026-11-14T10:00:04Z',
    completed_at: '2026-11-14T10:00:05Z',
    updated_at: '2026-11-14T10:00:05Z',
    type: 'assistant_message',
    message_id: 'm-1',
    text: 'Build passed.',
    streaming: false,
  };
  const streamA: TurnItem = {
    id: 'a-2',
    turn_id: 't-2',
    ordinal: 4,
    status: 'running',
    title: null,
    started_at: '2026-11-15T10:00:01Z',
    completed_at: null,
    updated_at: '2026-11-15T10:00:01Z',
    type: 'assistant_message',
    message_id: 'm-2',
    text: 'Part 1',
    streaming: true,
  };
  const streamB: TurnItem = { ...streamA, text: 'Part 1 + Part 2', updated_at: '2026-11-15T10:00:02Z' };

  const rows1 = timeline.timelineRows([u1, cmd1, a1, streamA], turns);
  const rows2 = timeline.timelineRows([u1, cmd1, a1, streamB], turns);

  assert.equal(rows1.length, rows2.length);
  // Day 1 separator, u1, cmd1 work row, a1, Day 2 separator, and agent_turn divider MUST all be strictly ===!
  for (let i = 0; i < rows1.length - 1; i++) {
    assert.equal(rows1[i], rows2[i], `expected row[${i}] (${rows1[i].kind}) to keep strict === reference`);
  }
  const workRow = rows1.find((r: { kind: string }) => r.kind === 'work');
  assert.ok(workRow);
  assert.equal(timeline.workRowEntries(workRow), timeline.workRowEntries(workRow));
  // Only the final streaming message row gets a new wrapper with updated text.
  assert.notEqual(rows1[rows1.length - 1], rows2[rows2.length - 1]);
  assert.equal(rows2[rows2.length - 1].item.text, 'Part 1 + Part 2');
});

test('RuntimeViews skips keydown replays, suppresses 4s liveness GET during active WS stream, and batches burst deltas', async () => {
  let nowMs = 100_000;
  const FakeDate = Object.assign(function (...args: unknown[]) {
    return new Date(...(args as [number]));
  }, { now: () => nowMs, parse: Date.parse, UTC: Date.UTC });
  const h = makeRuntimeHarness({
    Date: FakeDate,
    requestAnimationFrame: (cb: () => void) => setTimeout(cb, 16),
  });
  h.views.setSessionActiveProbe(() => true);
  const close = h.views.open('s-stream');
  h.gets[0].gate.resolve({ kind: 'events', events: [], last_sequence: 0 });
  await flush();
  assert.equal(h.gets.length, 1);

  // 1) Typing (`keydown` on window) MUST NOT trigger HTTP GET /events mid-keystroke.
  nowMs += 10_000;
  h.emitWindow('keydown');
  await flush();
  assert.equal(h.gets.length, 1, 'keydown must never trigger mid-typing replay');

  // 2) Streaming burst: first streaming event applies immediately (0ms); next 2 events within 32ms coalesce!
  const streamEv = (seq: number, text: string, streaming = true): RuntimeEvent => ({
    sequence: seq,
    session_id: 's-stream',
    ts: '2026-11-15T10:00:00Z',
    type: 'turn_item.updated',
    turn_item: {
      id: 'm-live',
      turn_id: 't-live',
      ordinal: 1,
      status: streaming ? 'running' : 'completed',
      title: null,
      started_at: '2026-11-15T10:00:00Z',
      completed_at: streaming ? null : '2026-11-15T10:00:01Z',
      updated_at: '2026-11-15T10:00:00Z',
      type: 'assistant_message',
      message_id: 'm-live',
      text,
      streaming,
    },
  });

  h.emitLive(streamEv(1, 'tok1'));
  assert.equal(h.views.projections['s-stream']?.lastSequence, 1, 'leading token applies in 0ms');

  nowMs += 5;
  h.emitLive(streamEv(2, 'tok1 tok2'));
  nowMs += 5;
  h.emitLive(streamEv(3, 'tok1 tok2 tok3'));
  assert.equal(h.views.projections['s-stream']?.lastSequence, 1, 'burst tokens within 32ms wait in batch');

  // 3) 4s liveness tick while WS event arrived 10ms ago MUST NOT fire redundant GET /events!
  const liveness = [...h.timers.values()].find((t) => t.ms === 4_000);
  assert.ok(liveness);
  liveness.fn();
  await flush();
  assert.equal(h.gets.length, 1, 'active WS stream suppresses redundant 4s HTTP replay');

  // 4) Non-streaming completion event flushes pending batch + completion immediately in 0ms!
  nowMs += 5;
  h.emitLive(streamEv(4, 'tok1 tok2 tok3 done', false));
  assert.equal(h.views.projections['s-stream']?.lastSequence, 4);
  assert.equal(
    (h.views.projections['s-stream']?.items[0] as Extract<TurnItem, { type: 'assistant_message' }>).text,
    'tok1 tok2 tok3 done',
  );

  // 5) Once WS goes quiet for >= 2.5s, 4s liveness tick resumes catching up as safety net.
  nowMs += 3_000;
  liveness.fn();
  await flush();
  assert.equal(h.gets.length, 2, 'liveness tick catches up once WS has been quiet >= 2.5s');
  h.gets[1].gate.resolve({ kind: 'events', events: [], last_sequence: 4 });
  await flush();

  close();
  assert.equal(h.timers.size, 0);
});

test('RuntimeViews mobile viewport requests tail_turns=12&max_lines=10000&compact=true and lazily hydrates + preserves expanded item details', async () => {
  const h = makeRuntimeHarness();
  h.winState.innerWidth = 390;
  const close = h.views.open('s-mobile');
  assert.equal(h.gets[0].path, '/runtime-sessions/s-mobile/events?after_sequence=0&tail_turns=12&max_lines=10000&compact=true');

  const compactEdit: TurnItem = {
    id: 'fc-1',
    turn_id: 't-1',
    ordinal: 1,
    status: 'completed',
    title: 'src/main.rs',
    started_at: '2026-11-15T10:00:00Z',
    completed_at: '2026-11-15T10:00:01Z',
    updated_at: '2026-11-15T10:00:01Z',
    detail_omitted: true,
    type: 'file_change',
    file_name: 'src/main.rs',
  };
  h.gets[0].gate.resolve({
    kind: 'snapshot',
    snapshot_sequence: 5,
    snapshot: {
      provider_session: null,
      provider_thread: null,
      turns: [
        {
          id: 't-2',
          ordinal: 2,
          status: 'completed',
          started_at: '2026-11-15T10:00:00Z',
          completed_at: '2026-11-15T10:00:02Z',
          origin: 'user',
        },
      ],
      turn_items: [{ ...compactEdit, turn_id: 't-2' }],
      runtime_requests: [],
      plans: [],
      queued_prompts: [],
      has_older: false,
      oldest_turn_ordinal: 1,
      older_turns: 0,
    },
  });
  await flush();
  assert.equal(h.views.projections['s-mobile']?.items[0]?.detail_omitted, true);

  // Concurrent calls coalesce into one GET /runtime-sessions/s-mobile/items/fc-1.
  const p1 = h.views.ensureItemDetail('s-mobile', 'fc-1');
  const p2 = h.views.ensureItemDetail('s-mobile', 'fc-1');
  assert.equal(h.gets.length, 2);
  assert.equal(h.gets[1].path, '/runtime-sessions/s-mobile/items/fc-1');
  assert.equal(h.views.loadingItem['s-mobile:fc-1'], true);

  const hydratedEdit: TurnItem = {
    ...compactEdit,
    detail_omitted: undefined,
    diff_str: '--- a/src/main.rs\n+++ b/src/main.rs\n@@ -1,1 +1,1 @@\n-old\n+new',
  };
  (h.gets[1].gate as unknown as { resolve(v: { item: TurnItem }): void }).resolve({ item: hydratedEdit });
  const [r1, r2] = await Promise.all([p1, p2]);
  assert.equal(r1?.id, 'fc-1');
  assert.equal(r2?.id, 'fc-1');
  assert.equal(h.views.loadingItem['s-mobile:fc-1'], undefined);
  const afterHydrate = h.views.projections['s-mobile']?.items[0] as Extract<TurnItem, { type: 'file_change' }>;
  assert.equal(afterHydrate.detail_omitted, undefined);
  assert.match(afterHydrate.diff_str ?? '', /\+new/);

  // Item hydration failure sets `itemError` for inline Retry, then clears on successful retry.
  const failItemP = h.views.ensureItemDetail('s-mobile', 'fc-err');
  (h.gets[2].gate as unknown as { reject(e: Error): void }).reject(new Error('item 502'));
  assert.equal(await failItemP, null);
  assert.match(h.views.itemError['s-mobile:fc-err'] ?? '', /item 502/);

  // Reconnect snapshot with unchanged updated_at preserves already-hydrated diff_str!
  h.emitResync();
  await flush();
  h.gets[3].gate.resolve({
    kind: 'snapshot',
    snapshot_sequence: 6,
    snapshot: {
      provider_session: null,
      provider_thread: null,
      turns: [
        {
          id: 't-2',
          ordinal: 2,
          status: 'completed',
          started_at: '2026-11-15T10:00:00Z',
          completed_at: '2026-11-15T10:00:02Z',
          origin: 'user',
        },
      ],
      turn_items: [{ ...compactEdit, turn_id: 't-2' }],
      runtime_requests: [],
      plans: [],
      queued_prompts: [],
      has_older: true,
      oldest_turn_ordinal: 2,
      older_turns: 1,
    },
  });
  await flush();
  const afterResync = h.views.projections['s-mobile']?.items[0] as Extract<TurnItem, { type: 'file_change' }>;
  assert.equal(afterResync.detail_omitted, undefined);
  assert.match(afterResync.diff_str ?? '', /\+new/);

  // `loadOlder` failure records inline `olderError` and clears on Retry.
  const failOlderP = h.views.loadOlder('s-mobile');
  h.gets[4].gate.reject(new Error('history timeout'));
  assert.equal(await failOlderP, false);
  assert.match(h.views.olderError['s-mobile'] ?? '', /history timeout/);

  const okOlderP = h.views.loadOlder('s-mobile');
  h.gets[5].gate.resolve({
    kind: 'snapshot',
    snapshot_sequence: 6,
    snapshot: {
      provider_session: null,
      provider_thread: null,
      turns: [],
      turn_items: [],
      runtime_requests: [],
      plans: [],
      queued_prompts: [],
      has_older: false,
      oldest_turn_ordinal: 1,
      older_turns: 0,
    },
  });
  assert.equal(await okOlderP, true);
  assert.equal(h.views.olderError['s-mobile'], undefined);

  close();
});

test('projection.ts caps streaming output at 128 KiB, drops settled duplicate output, batches applyEvents, and guards non-contiguous applyReplay gaps', () => {
  const projMod = loadSource(
    new URL('../src/modules/threads/runtime/projection.ts', import.meta.url),
    {},
  ) as typeof import('../src/modules/threads/runtime/projection.ts');

  assert.equal(projMod.MAX_STREAM_OUTPUT_BYTES, 128 * 1024);

  // 1) Streaming output caps at 128 KiB retaining newest tail, then deletes transient entry once settled item carries output.
  const chunkA = 'A'.repeat(90 * 1024);
  const chunkB = 'B'.repeat(60 * 1024) + 'TAIL_END';
  const { projection: p1, gap } = projMod.applyEvents(projMod.emptyProjection(), [
    {
      sequence: 1,
      session_id: 's-cap',
      ts: '2026-11-15T10:00:00Z',
      type: 'command_output.delta',
      item_id: 'cmd-1',
      chunk: chunkA,
    },
    {
      sequence: 2,
      session_id: 's-cap',
      ts: '2026-11-15T10:00:01Z',
      type: 'command_output.delta',
      item_id: 'cmd-1',
      chunk: chunkB,
    },
  ]);
  assert.equal(gap, false);
  assert.equal(p1.output['cmd-1'].length, projMod.MAX_STREAM_OUTPUT_BYTES);
  assert.ok(p1.output['cmd-1'].startsWith('…[truncated]\n'));
  assert.ok(p1.output['cmd-1'].endsWith('TAIL_END'));

  const { projection: p2 } = projMod.applyEvents(p1, [
    {
      sequence: 3,
      session_id: 's-cap',
      ts: '2026-11-15T10:00:02Z',
      type: 'turn_item.updated',
      turn_item: {
        id: 'cmd-1',
        turn_id: 't-1',
        ordinal: 1,
        status: 'completed',
        title: 'cargo test',
        started_at: '2026-11-15T10:00:00Z',
        completed_at: '2026-11-15T10:00:02Z',
        updated_at: '2026-11-15T10:00:02Z',
        type: 'command_execution',
        input: 'cargo test',
        output: 'settled output',
        exit_code: 0,
      },
    },
  ]);
  assert.equal('cmd-1' in p2.output, false, 'settled turn_item with output must release transient delta buffer');

  // 2) `replaceHydratedItem` preserves existing `diff_str` when hydrated detail lacks `diff_str`.
  const withDiff = projMod.replaceHydratedItem(projMod.emptyProjection(), {
    id: 'fc-keep',
    turn_id: 't-1',
    ordinal: 1,
    status: 'completed',
    title: 'lib.rs',
    started_at: '2026-11-15T10:00:00Z',
    completed_at: '2026-11-15T10:00:01Z',
    updated_at: '2026-11-15T10:00:01Z',
    type: 'file_change',
    file_name: 'lib.rs',
    diff_str: '+preserved line',
  });
  const afterMissingDiff = projMod.replaceHydratedItem(withDiff, {
    id: 'fc-keep',
    turn_id: 't-1',
    ordinal: 1,
    status: 'completed',
    title: 'lib.rs',
    started_at: '2026-11-15T10:00:00Z',
    completed_at: '2026-11-15T10:00:01Z',
    updated_at: '2026-11-15T10:00:01Z',
    type: 'file_change',
    file_name: 'lib.rs',
  });
  assert.equal(
    (afterMissingDiff.items[0] as Extract<TurnItem, { type: 'file_change' }>).diff_str,
    '+preserved line',
  );

  // 3) Non-contiguous `applyReplay` snapshot drops stale older turns instead of creating an ordinal hole [1, 2, 5, 6].
  const mkT = (ordinal: number): ProviderTurn => ({
    id: `t-${ordinal}`,
    ordinal,
    status: 'completed',
    started_at: '2026-11-15T10:00:00Z',
    completed_at: '2026-11-15T10:00:01Z',
    origin: 'user',
  });
  const stalePaged = projMod.fromSnapshot(
    {
      provider_session: null,
      provider_thread: null,
      turns: [mkT(1), mkT(2)],
      turn_items: [],
      runtime_requests: [],
      plans: [],
      queued_prompts: [],
      has_older: false,
      oldest_turn_ordinal: 1,
      older_turns: 0,
    },
    2,
  );
  const gappedTail = projMod.applyReplay(stalePaged, {
    kind: 'snapshot',
    snapshot_sequence: 10,
    snapshot: {
      provider_session: null,
      provider_thread: null,
      turns: [mkT(5), mkT(6)],
      turn_items: [],
      runtime_requests: [],
      plans: [],
      queued_prompts: [],
      has_older: true,
      oldest_turn_ordinal: 5,
      older_turns: 4,
    },
  });
  assert.deepEqual(
    Array.from(gappedTail.turns, (t) => t.ordinal),
    [5, 6],
  );
  assert.equal(gappedTail.hasOlder, true);
  assert.equal(gappedTail.oldestTurnOrdinal, 5);
});

test('Go LSP init/dedupe, Go unused-import QuickFix guard, inspection hover suppression, and prod bridge detection', async () => {
  const lspComp = await import('../src/lib/components/cm-ide-lsp-completion.ts');
  const qfMod = await import('../src/lib/components/cm-ide-quickfix.ts');
  const hoverMod = await import('../src/lib/components/cm-ide-hover.ts');
  const bridgeMod = loadSource(
    new URL('../src/modules/ide/shell/networkBridgeModel.ts', import.meta.url),
    {},
  ) as typeof import('../src/modules/ide/shell/networkBridgeModel.ts');

  // 1) Go LSP `initializationOptions` passes `-mod=mod` and resolves sub-repo module root.
  const goInit = lspComp.buildLspInitializationOptions('go');
  assert.deepEqual(goInit?.buildFlags, ['-mod=mod']);
  assert.equal(
    lspComp.resolveIdeLspModuleRoot('/ws/multi', 'services/billing/cmd/main.go', [
      '/ws/multi/services/auth',
      '/ws/multi/services/billing',
    ]),
    '/ws/multi/services/billing',
  );

  // 2) `dedupeLspJsonRpcMessage` collapses duplicate `compiler(BrokenImport)` diagnostics for regular + test pkg variants.
  const dupDiag = {
    range: { start: { line: 4, character: 2 }, end: { line: 4, character: 44 } },
    severity: 1,
    code: 'BrokenImport',
    source: 'compiler',
    message: 'could not import bitbucket.org/gamescale-rnd/go_utilities/web',
  };
  const dedupedMsg = lspComp.dedupeLspJsonRpcMessage({
    jsonrpc: '2.0',
    method: 'textDocument/publishDiagnostics',
    params: { uri: 'file:///ws/main.go', diagnostics: [dupDiag, { ...dupDiag }] },
  });
  assert.equal(dedupedMsg.params?.diagnostics?.length, 1);

  // 3) Go `Remove import` only appears when the imported package is genuinely unused (or flagged unused by LSP).
  const usedGoDoc = [
    'package main',
    'import (',
    '\t"bitbucket.org/gamescale-rnd/go_utilities/web"',
    '\t"unused/pkg/foo"',
    '\t_ "embed"',
    ')',
    'func main() { web.InitApp() }',
  ].join('\n');
  assert.equal(qfMod.isGoImportUnusedInDoc(usedGoDoc, 3), false, 'used `web` import must NOT offer Remove import');
  assert.equal(qfMod.isGoImportUnusedInDoc(usedGoDoc, 4), true, 'unreferenced `foo` import SHOULD offer Remove import');
  assert.equal(qfMod.isGoImportUnusedInDoc(usedGoDoc, 5), false, 'blank `_` import must NOT offer Remove import');

  // 4) Hover never synthesizes an inspection card from `primaryDetFix` alone or over an active `@codemirror/lint` range.
  assert.equal(
    hoverMod.shouldShowInspectionHoverFallback({ hasParsedDoc: false, hasPrimaryLineFix: false, hasLintAtPos: false }),
    false,
  );
  assert.equal(
    hoverMod.shouldShowInspectionHoverFallback({ hasParsedDoc: false, hasPrimaryLineFix: true, hasLintAtPos: true }),
    false,
  );
  assert.equal(
    hoverMod.shouldShowInspectionHoverFallback({ hasParsedDoc: false, hasPrimaryLineFix: true, hasLintAtPos: false }),
    true,
  );

  // 5) `isProdEnvironment` catches `prod*` / `production` environments for danger bridge styling & confirmation.
  assert.equal(bridgeMod.isProdEnvironment('prod-eu'), true);
  assert.equal(bridgeMod.isProdEnvironment('Production'), true);
  assert.equal(bridgeMod.isProdEnvironment('stg-m'), false);
  assert.equal(bridgeMod.isProdEnvironment(null), false);
});

/** Timers created by `act` (the chase timer, not the liveness poll). */
async function newTimers(h: ReturnType<typeof makeRuntimeHarness>, act: () => void) {
  const before = new Set(h.timers.keys());
  act();
  await flush();
  return [...h.timers.entries()].filter(([id]) => !before.has(id)).map(([, t]) => t);
}

test('RuntimeViews catch-up: an announced sequence never delivered backs off, is bounded, then reloads one snapshot', async () => {
  const h = makeRuntimeHarness();
  const close = h.views.open('s-c');
  h.gets[0].gate.resolve({ kind: 'events', events: [msgEvent('s-c', 1, 'm-1', 'a'), msgEvent('s-c', 2, 'm-2', 'b')], last_sequence: 2 });
  await flush();
  assert.equal(h.gets.length, 1);

  // The session row announces 9; the events endpoint only ever has 2.
  h.views.noteSessionSequence('s-c', 9);
  await flush();
  assert.equal(h.gets.length, 2);
  const delays: number[] = [];
  let pending = h.gets[1];
  for (let i = 0; i < 6; i++) {
    const made = await newTimers(h, () => pending.gate.resolve({ kind: 'events', events: [], last_sequence: 2 }));
    assert.equal(made.length, 1, 'no immediate refetch: one backoff timer');
    delays.push(made[0].ms);
    const n: number = h.gets.length;
    made[0].fn();
    await flush();
    assert.equal(h.gets.length, n + 1);
    pending = h.gets[h.gets.length - 1];
  }
  assert.deepEqual(delays, [250, 500, 1_000, 2_000, 4_000, 4_000]);
  const eventFetches = h.gets.slice(1).filter((g) => !g.path.includes('after_sequence=0'));
  assert.equal(eventFetches.length, 6, 'bounded: the first catch-up plus five backoff attempts');
  assert.match(pending.path, /after_sequence=0&/, 'then one snapshot reload');

  // The snapshot is the daemon's truth: chasing stops.
  const after = await newTimers(h, () =>
    pending.gate.resolve({
      kind: 'snapshot',
      snapshot_sequence: 2,
      snapshot: { provider_session: null, provider_thread: null, turns: [], turn_items: [], runtime_requests: [], plans: [], queued_prompts: [] },
    } as unknown as RuntimeReplay),
  );
  assert.equal(after.length, 0);
  const total = h.gets.length;
  await flush();
  assert.equal(h.gets.length, total, 'no further fetches');
  close();
  assert.equal(h.timers.size, 0);
});

test('RuntimeViews catch-up: progress resets the backoff and keeps paging at once', async () => {
  const h = makeRuntimeHarness();
  const close = h.views.open('s-p');
  h.gets[0].gate.resolve({ kind: 'events', events: [msgEvent('s-p', 1, 'm-1', 'a')], last_sequence: 1 });
  await flush();
  h.views.noteSessionSequence('s-p', 9);
  await flush();
  let made = await newTimers(h, () => h.gets[1].gate.resolve({ kind: 'events', events: [], last_sequence: 1 }));
  assert.deepEqual(made.map((t) => t.ms), [250]);
  made[0].fn();
  await flush();
  made = await newTimers(h, () => h.gets[2].gate.resolve({ kind: 'events', events: [], last_sequence: 1 }));
  assert.deepEqual(made.map((t) => t.ms), [500]);
  made[0].fn();
  await flush();
  // A page arrives (progress): the next fetch goes out at once, no timer.
  made = await newTimers(h, () => h.gets[3].gate.resolve({ kind: 'events', events: [msgEvent('s-p', 2, 'm-2', 'b')], last_sequence: 2 }));
  assert.equal(made.length, 0);
  assert.equal(h.gets.length, 5);
  assert.match(h.gets[4].path, /after_sequence=2&/);
  // Stuck again: the backoff starts over at 250 ms.
  made = await newTimers(h, () => h.gets[4].gate.resolve({ kind: 'events', events: [], last_sequence: 2 }));
  assert.deepEqual(made.map((t) => t.ms), [250]);
  close();
});
