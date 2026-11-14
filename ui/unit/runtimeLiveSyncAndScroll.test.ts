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

function makeRuntimeHarness() {
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
  const winState = {
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
    },
    { relative: true },
  );

  return {
    views: new mod.RuntimeViews() as {
      projections: Record<string, { lastSequence: number; items: TurnItem[] }>;
      loadError: Record<string, string>;
      staleError: Record<string, string>;
      open(id: string): () => void;
      retry(id: string): void;
      noteSessionSequence(id: string, seq: number | null | undefined): void;
      setSessionActiveProbe(fn: (id: string) => boolean): void;
    },
    gets,
    timers,
    docState,
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
  assert.equal(h.gets[0].path, '/runtime-sessions/s-1/events?after_sequence=0');
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
  assert.equal(h.gets[1].path, '/runtime-sessions/s-1/events?after_sequence=3');
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
  assert.equal(h.gets[2].path, '/runtime-sessions/s-2/events?after_sequence=1');
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
  assert.equal(h.gets[4].path, '/runtime-sessions/s-2/events?after_sequence=3');
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
    plain(live.deriveLiveFooter(proj, null, false, [{ label: 'Timer (30s)' }, { label: 'Swarm board wait' }])),
    {
      mode: 'waiting_bg',
      text: 'Waiting on 2 background tasks: Timer (30s), Swarm board wait — new messages will appear automatically',
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
    '../../lib/providers': { harnessLabel: providers.harnessLabel },
  });
  // When `preferred` ('pi') cannot carry a persona AND `claude` is disabled, picks first enabled persona CLI (`codex`).
  assert.equal(persona.personaDefault('pi', ['codex', 'opencode', 'pi']), 'codex');
  assert.equal(persona.personaDefault('opencode', ['codex', 'opencode', 'pi']), 'opencode');
});
