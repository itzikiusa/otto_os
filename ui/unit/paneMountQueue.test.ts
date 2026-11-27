import { test } from 'node:test';
import assert from 'node:assert/strict';
import { mkdtempSync, readFileSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';
import ts from 'typescript';
import { compileModule } from 'svelte/compiler';
import { componentFunctions } from './componentFunctions.ts';
import { svelteClient } from './runesHarness.ts';
import {
  INITIAL_MOUNT_DELAY_MS,
  PaneMountQueue,
  STEP_MOUNT_DELAY_MS,
  selectLiveTiles,
  shouldHandOffPlaceholderFocus,
  type PaneMountQueueOpts,
} from '../src/modules/agents/paneMountQueue.ts';

function fakeClock() {
  let now = 0;
  let seq = 0;
  const timers = new Map<number, { at: number; fn: () => void }>();
  const setT = (fn: () => void, ms: number) => {
    timers.set(++seq, { at: now + ms, fn });
    return seq as unknown as ReturnType<typeof setTimeout>;
  };
  const clearT = (id: ReturnType<typeof setTimeout>) => {
    timers.delete(id as unknown as number);
  };
  const advance = (ms: number) => {
    const target = now + ms;
    while (true) {
      let earliestId: number | null = null;
      let earliestAt = Infinity;
      for (const [id, t] of timers) {
        if (t.at <= target && t.at < earliestAt) {
          earliestAt = t.at;
          earliestId = id;
        }
      }
      if (earliestId === null) break;
      const entry = timers.get(earliestId)!;
      timers.delete(earliestId);
      now = entry.at;
      entry.fn();
    }
    now = target;
  };
  return { setT, clearT, advance, pending: () => timers.size };
}

test('paneMountQueue: Frame 0 mounts ONLY the priority pane regardless of N (20 panes)', () => {
  const clock = fakeClock();
  const snapshots: string[][] = [];
  const q = new PaneMountQueue((s) => snapshots.push([...s]), { setT: clock.setT, clearT: clock.clearT, isVisible: () => true });
  const wanted = Array.from({ length: 20 }, (_, i) => `p${i}`);

  q.sync(wanted, 'p7');
  assert.deepEqual(snapshots, [['p7']], 'only the priority pane mounts synchronously');

  // Before INITIAL_MOUNT_DELAY_MS, still only 1 pane mounted.
  clock.advance(INITIAL_MOUNT_DELAY_MS - 1);
  assert.equal(snapshots.length, 1);

  // At INITIAL_MOUNT_DELAY_MS, the first background pane hydrates.
  clock.advance(1);
  assert.deepEqual(snapshots.at(-1), ['p7', 'p0']);

  // Each STEP_MOUNT_DELAY_MS hydrates one more pane in order.
  clock.advance(STEP_MOUNT_DELAY_MS);
  assert.deepEqual(snapshots.at(-1), ['p7', 'p0', 'p1']);

  // Clicking/focusing an unmounted pane promotes it immediately without dropping prior panes.
  q.sync(wanted, 'p15');
  assert.deepEqual(snapshots.at(-1), ['p7', 'p0', 'p1', 'p15']);

  // Reset (workspace switch) cancels pending timers and drops mounted state so hidden panes cost nothing.
  q.reset();
  assert.deepEqual(snapshots.at(-1), []);
  assert.equal(clock.pending(), 0, 'all background mount timers cancelled on reset');
});

test('paneMountQueue: pauses background hydration while tab is hidden and resumes on wake', () => {
  const clock = fakeClock();
  let visible = false;
  let latest: string[] = [];
  const q = new PaneMountQueue((s) => (latest = [...s]), {
    setT: clock.setT,
    clearT: clock.clearT,
    isVisible: () => visible,
  });

  q.sync(['a', 'b', 'c'], 'a');
  assert.deepEqual(latest, ['a']);
  clock.advance(1000);
  assert.deepEqual(latest, ['a'], 'no background mounts while hidden');

  visible = true;
  q.wake();
  clock.advance(INITIAL_MOUNT_DELAY_MS);
  assert.deepEqual(latest, ['a', 'b']);
  clock.advance(STEP_MOUNT_DELAY_MS);
  assert.deepEqual(latest, ['a', 'b', 'c']);
  assert.equal(clock.pending(), 0);
});

test('paneMountQueue: prunes panes removed from wanted (scrolled off / closed)', () => {
  const clock = fakeClock();
  let latest: string[] = [];
  const q = new PaneMountQueue((s) => (latest = [...s]), { setT: clock.setT, clearT: clock.clearT, isVisible: () => true });

  q.sync(['a', 'b', 'c'], 'a');
  clock.advance(INITIAL_MOUNT_DELAY_MS);
  assert.deepEqual(latest, ['a', 'b']);

  q.sync(['a', 'c'], 'a');
  assert.deepEqual(latest, ['a']);
  clock.advance(INITIAL_MOUNT_DELAY_MS);
  assert.deepEqual(latest, ['a', 'c']);
  q.destroy();
  assert.equal(clock.pending(), 0);
});

test('selectLiveTiles: skips inactive (reconnectable/exited) and thread tiles in auto-visible pass, keeps active & pinned', () => {
  const items = [
    ...Array.from({ length: 11 }, (_, i) => ({ id: `live-${i}`, inactive: false, thread: false })),
    ...Array.from({ length: 10 }, (_, i) => ({ id: `rec-${i}`, inactive: true, thread: false })),
    { id: 'thr-0', inactive: false, thread: true },
  ];
  const visible = new Set(items.map((x) => x.id));
  const pinned = new Set(['rec-2']);

  const selected = selectLiveTiles(items, {
    activeId: 'live-3',
    pinned,
    visible,
    isInactive: (x) => x.inactive,
    isThread: (x) => x.thread,
    maxLive: 15,
  });

  assert.equal(selected[0], 'live-3', 'active tile is first');
  assert.equal(selected[1], 'rec-2', 'explicitly pinned suspended tile is honored');
  assert.equal(selected.length, 12, '11 live + 1 pinned = 12 (9 unpinned reconnectable + 1 thread skipped)');
  assert.equal(selected.includes('rec-0'), false, 'unpinned reconnectable tiles stay placeholders');
  assert.equal(selected.includes('thr-0'), false, 'thread tiles do not consume terminal slots');
});

test('selectLiveTiles: retains an already-mounted visible tile across exit until scrolled off (OBS-1)', () => {
  const items = [
    { id: 'live-0', inactive: false, thread: false },
    { id: 'live-1', inactive: true, thread: false }, // was live & mounted, just exited nonzero
    { id: 'cold-exited', inactive: true, thread: false }, // already exited on cold open
  ];
  const visible = new Set(['live-0', 'live-1', 'cold-exited']);
  const retained = new Set(['live-0', 'live-1']);

  const afterExit = selectLiveTiles(items, {
    activeId: 'live-0',
    pinned: new Set(),
    visible,
    retained,
    isInactive: (x) => x.inactive,
    isThread: (x) => x.thread,
    maxLive: 15,
  });
  assert.deepEqual(afterExit, ['live-0', 'live-1'], 'already-mounted visible tile stays live across exit; cold exited stays placeholder');

  // Once scrolled off-screen, it is evicted normally.
  visible.delete('live-1');
  const scrolledOff = selectLiveTiles(items, {
    activeId: 'live-0',
    pinned: new Set(),
    visible,
    retained,
    isInactive: (x) => x.inactive,
    isThread: (x) => x.thread,
    maxLive: 15,
  });
  assert.deepEqual(scrolledOff, ['live-0'], 'scrolled-off exited tile is evicted');

  // Scrolling it back after unmount does not cold-attach an exited session.
  visible.add('live-1');
  retained.delete('live-1');
  const scrolledBack = selectLiveTiles(items, {
    activeId: 'live-0',
    pinned: new Set(),
    visible,
    retained,
    isInactive: (x) => x.inactive,
    isThread: (x) => x.thread,
    maxLive: 15,
  });
  assert.deepEqual(scrolledBack, ['live-0'], 'unmounted exited tile stays placeholder if scrolled back');
});

test('TiledView.attach: clicking a queued tile in a multi-leaf split keeps focus on its actual leaf (BUG-17)', () => {
  const leaves = ['s0', 's1', 's2'];
  let focusedIdx = 0;
  let saved = 0;
  const ctx = componentFunctions(new URL('../src/modules/agents/TiledView.svelte', import.meta.url), ['attach'], {
    pinned: new Set<string>(),
    savePinned: () => { saved++; },
    ws: {
      openSession: (id: string) => {
        const i = leaves.indexOf(id);
        if (i >= 0) focusedIdx = i;
      },
    },
    layout: {
      focusIndex: (i: number) => { focusedIdx = i; },
    },
  });

  ctx.attach('s2');
  assert.equal(focusedIdx, 2, 'focus must stay on leaf 2, not reset to leaf 0');
  assert.equal(ctx.pinned.has('s2'), true, 'clicked tile is pinned');
  assert.equal(saved, 1);
});

test('SplitNode placeholder hydration: transfers keyboard focus only when placeholder owned focus (FE-2)', () => {
  const child = {};
  const placeholder = { contains: (el: unknown) => el === child };
  const other = {};
  assert.equal(shouldHandOffPlaceholderFocus(true, placeholder, placeholder), true, 'direct focus on placeholder transfers');
  assert.equal(shouldHandOffPlaceholderFocus(true, placeholder, child), true, 'descendant focus inside placeholder transfers');
  assert.equal(shouldHandOffPlaceholderFocus(true, placeholder, other), false, 'focus elsewhere is never stolen');
  assert.equal(shouldHandOffPlaceholderFocus(false, placeholder, placeholder), false, 'unmounted placeholder keeps focus');
  assert.equal(shouldHandOffPlaceholderFocus(true, null, placeholder), false, 'missing placeholder does not transfer');

  // Drive SplitNode's actual capturePlaceholderFocus / finishPlaceholderFocus handlers.
  let focusedTarget: string | null = null;
  const paneTitle = { focus: () => { focusedTarget = 'pane-title'; } };
  const doc: { activeElement: unknown } = { activeElement: placeholder };
  const leafEl = {
    contains: (el: unknown) => el === paneTitle,
    querySelector: () => paneTitle,
  };
  const ctx = componentFunctions(
    new URL('../src/modules/agents/SplitNode.svelte', import.meta.url),
    ['capturePlaceholderFocus', 'finishPlaceholderFocus'],
    {
      shouldHandOffPlaceholderFocus,
      sessionMounted: true,
      pendingBtn: placeholder,
      restorePlaceholderFocus: false,
      leafEl,
      document: doc,
    },
  );
  // 1) Tabbed onto placeholder button, then hydration swaps it out (activeElement drops to body).
  ctx.capturePlaceholderFocus();
  doc.activeElement = other;
  ctx.finishPlaceholderFocus();
  assert.equal(focusedTarget, 'pane-title', 'hydrated pane receives focus when placeholder owned it');

  // 2) Background pane hydrates while another pane has focus -> must NOT steal focus.
  focusedTarget = null;
  doc.activeElement = other;
  ctx.capturePlaceholderFocus();
  ctx.finishPlaceholderFocus();
  assert.equal(focusedTarget, null, 'background hydration must not steal focus from another pane');
});

async function importPaneMountRunesDev() {
  const modPath = new URL('../src/modules/agents/paneMountQueue.svelte.ts', import.meta.url);
  const depUrl = new URL('../src/modules/agents/paneMountQueue.ts', import.meta.url).href;
  const svelteRoot = new URL('../node_modules/svelte/src/', import.meta.url);
  const clientMap: Record<string, string> = {
    svelte: new URL('index-client.js', svelteRoot).href,
    'svelte/internal/client': new URL('internal/client/index.js', svelteRoot).href,
    'svelte/internal/disclose-version': new URL('internal/disclose-version.js', svelteRoot).href,
    'svelte/internal/flags/async': new URL('internal/flags/async.js', svelteRoot).href,
    'svelte/internal/flags/legacy': new URL('internal/flags/legacy.js', svelteRoot).href,
    './paneMountQueue': depUrl,
  };
  const source = readFileSync(modPath, 'utf8');
  const js = ts.transpileModule(source, {
    compilerOptions: { module: ts.ModuleKind.ESNext, target: ts.ScriptTarget.ES2022, verbatimModuleSyntax: false },
  }).outputText;
  // Compile with dev: true so Svelte 5's `derived_references_self` guard is active.
  const out = compileModule(js, { filename: fileURLToPath(modPath).replace(/\.ts$/, '.js'), generate: 'client', dev: true }).js.code.replace(
    /from\s+['"]([^'"]+)['"]/g,
    (m, spec: string) => (clientMap[spec] ? `from '${clientMap[spec]}'` : m),
  );
  const dir = mkdtempSync(join(tmpdir(), 'otto-pane-mount-dev-'));
  const file = join(dir, 'paneMountQueue.svelte.mjs');
  writeFileSync(file, out);
  return (await import(pathToFileURL(file).href)) as {
    createTiledMountState: <T extends { id: string }>(opts: {
      getWorkspaceId: () => string | null;
      getOrdered: () => readonly T[];
      getActiveId: () => string | null;
      getPinned: () => ReadonlySet<string>;
      getVisible: () => ReadonlySet<string>;
      isThread: (item: T) => boolean;
      isInactive: (item: T) => boolean;
      maxLive: number;
      queueOpts?: PaneMountQueueOpts;
    }) => {
      readonly liveOrder: readonly string[];
      readonly liveIds: ReadonlySet<string>;
      readonly mountedIds: ReadonlySet<string>;
    };
  };
}

test('createTiledMountState (Svelte 5 dev runtime): cold mount is acyclic and retains exited mounted tiles until scrolled off (BUG-18, OBS-1)', async () => {
  const { createTiledMountState } = await importPaneMountRunesDev();
  const $ = await svelteClient();
  const clock = fakeClock();

  type Tile = { id: string; inactive: boolean };
  const wsId = $.state<string | null>('ws-1');
  const ordered = $.state<Tile[]>([
    { id: 'live-0', inactive: false },
    { id: 'live-1', inactive: false },
    { id: 'cold-exited', inactive: true },
  ]);
  const activeId = $.state<string | null>('live-0');
  const pinned = $.state<ReadonlySet<string>>(new Set());
  const visible = $.state<ReadonlySet<string>>(new Set(['live-0', 'live-1', 'cold-exited']));

  let state!: ReturnType<typeof createTiledMountState<Tile>>;
  const stop = $.effectRoot(() => {
    state = createTiledMountState({
      getWorkspaceId: () => $.get(wsId),
      getOrdered: () => $.get(ordered),
      getActiveId: () => $.get(activeId),
      getPinned: () => $.get(pinned),
      getVisible: () => $.get(visible),
      isThread: () => false,
      isInactive: (t) => t.inactive,
      maxLive: 15,
      queueOpts: { setT: clock.setT, clearT: clock.clearT, isVisible: () => true },
    });
    // Reading mountedIds & liveOrder on cold evaluation must not throw derived_references_self.
    assert.deepEqual([...state.liveOrder], ['live-0', 'live-1']);
    assert.deepEqual([...state.mountedIds], ['live-0']);
  });
  $.flushSync();

  assert.deepEqual([...state.liveOrder], ['live-0', 'live-1'], 'cold exited tile stays placeholder');
  assert.deepEqual([...state.mountedIds], ['live-0'], 'Frame 0 mounts only priority tile');

  // Hydrate background tile live-1.
  clock.advance(INITIAL_MOUNT_DELAY_MS);
  $.flushSync();
  assert.deepEqual([...state.mountedIds], ['live-0', 'live-1'], 'live-1 hydrates after initial grace');

  // live-1 exits nonzero while visible and unpinned: stays mounted so failure overlay & Restart survive.
  $.set(ordered, [
    { id: 'live-0', inactive: false },
    { id: 'live-1', inactive: true },
    { id: 'cold-exited', inactive: true },
  ]);
  $.flushSync();
  assert.deepEqual([...state.liveOrder], ['live-0', 'live-1'], 'exited mounted tile remains in liveOrder');
  assert.deepEqual([...state.mountedIds], ['live-0', 'live-1'], 'exited mounted tile remains mounted');

  // Scrolling live-1 off-screen evicts it cleanly.
  $.set(visible, new Set(['live-0', 'cold-exited']));
  $.flushSync();
  assert.deepEqual([...state.liveOrder], ['live-0']);
  assert.deepEqual([...state.mountedIds], ['live-0']);

  // Scrolling live-1 back while still exited does not cold-attach it.
  $.set(visible, new Set(['live-0', 'live-1', 'cold-exited']));
  $.flushSync();
  assert.deepEqual([...state.liveOrder], ['live-0']);
  assert.deepEqual([...state.mountedIds], ['live-0']);

  // Workspace switch clears admitted state and cancels timers.
  $.set(wsId, 'ws-2');
  $.flushSync();
  stop();
  assert.equal(clock.pending(), 0, 'all pending timers cleared on teardown');
});
