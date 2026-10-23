/// <reference lib="dom" />
// ui-performance-6-01 / maintainability-6-06 / architecture-5-05r: the three
// 3D surfaces (the classroom, the studio embed, Scene3DViewport) request
// frames only while there is work, and pause through the shared
// lib/three/stage.ts `watchVisibility`.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { demandLoop, watchVisibility } from '../src/lib/three/stage.ts';
import { schoolHasWork } from '../src/modules/home/school/scene/loop.ts';
import { embedHasWork } from '../src/modules/design-hall/studio3d/loop.ts';
import { viewportHasWork, viewportOwnsKey } from '../src/lib/design/scene3d/viewportLoop.ts';

/** A manual rAF: `flush()` runs the scheduled callbacks once. */
function fakeRaf() {
  let next = 1;
  const queue = new Map<number, (now: number) => void>();
  return {
    raf: (cb: (now: number) => void) => {
      const id = next++;
      queue.set(id, cb);
      return id;
    },
    caf: (id: number) => void queue.delete(id),
    get scheduled() {
      return queue.size;
    },
    flush(now = 0) {
      const due = [...queue.values()];
      queue.clear();
      for (const cb of due) cb(now);
    },
  };
}

test('demandLoop: an idle frame leaves nothing scheduled; kick restarts it', () => {
  const r = fakeRaf();
  let work = 3;
  let frames = 0;
  const loop = demandLoop(() => {
    frames++;
    return --work > 0;
  }, r);
  assert.equal(r.scheduled, 0, 'nothing runs before a kick');
  loop.kick();
  loop.kick(); // coalesced
  assert.equal(r.scheduled, 1);
  for (let i = 0; i < 10; i++) r.flush();
  assert.equal(frames, 3, 'stops re-arming once the work is done');
  assert.equal(r.scheduled, 0);
  assert.equal(loop.pending, false);
  work = 1;
  loop.kick();
  r.flush();
  assert.equal(frames, 4);
  assert.equal(r.scheduled, 0);
});

test('demandLoop: canRun gates kicks and frames; stop cancels; pending covers a running frame', () => {
  const r = fakeRaf();
  let ok = false;
  let seenPending = false;
  const loop = demandLoop(() => {
    seenPending = loop.pending;
    return true;
  }, { ...r, canRun: () => ok });
  loop.kick();
  assert.equal(r.scheduled, 0, 'a paused loop schedules nothing');
  ok = true;
  loop.kick();
  r.flush();
  assert.equal(seenPending, true);
  assert.equal(r.scheduled, 1, 'work re-arms');
  ok = false;
  r.flush();
  assert.equal(r.scheduled, 0, 'a frame that may not run does not re-arm');
  ok = true;
  loop.kick();
  loop.stop();
  assert.equal(r.scheduled, 0);
  assert.equal(loop.pending, false);
});

test('watchVisibility: emits the initial state once, then only on flips of in-view AND document-visible', () => {
  const doc = new EventTarget() as EventTarget & { hidden: boolean };
  doc.hidden = false;
  let ioCb: ((entries: { isIntersecting: boolean }[]) => void) | null = null;
  let disconnected = false;
  const g = globalThis as Record<string, unknown>;
  const saved = { document: g.document, IntersectionObserver: g.IntersectionObserver };
  g.document = doc;
  g.IntersectionObserver = class {
    constructor(cb: (entries: { isIntersecting: boolean }[]) => void) {
      ioCb = cb;
    }
    observe() {}
    disconnect() {
      disconnected = true;
    }
  };
  try {
    const seen: boolean[] = [];
    const off = watchVisibility({} as Element, (v) => seen.push(v));
    assert.deepEqual(seen, [true]);
    ioCb!([{ isIntersecting: true }]);
    assert.deepEqual(seen, [true], 'no flip, no call');
    ioCb!([{ isIntersecting: false }]);
    assert.deepEqual(seen, [true, false]);
    doc.hidden = true;
    doc.dispatchEvent(new Event('visibilitychange'));
    assert.deepEqual(seen, [true, false], 'still not drawable');
    ioCb!([{ isIntersecting: true }]);
    assert.deepEqual(seen, [true, false], 'in view but hidden');
    doc.hidden = false;
    doc.dispatchEvent(new Event('visibilitychange'));
    assert.deepEqual(seen, [true, false, true]);
    off();
    assert.equal(disconnected, true);
    doc.hidden = true;
    doc.dispatchEvent(new Event('visibilitychange'));
    assert.deepEqual(seen, [true, false, true], 'unsubscribed');
  } finally {
    g.document = saved.document;
    g.IntersectionObserver = saved.IntersectionObserver;
  }
});

test('school loop: an idle corridor or a reduced-motion room has no work', () => {
  const idle = { tween: false, keys: 0, roomOpen: false, reduced: false, kicks: 0, doorsMoving: false };
  assert.equal(schoolHasWork(idle), false);
  assert.equal(schoolHasWork({ ...idle, roomOpen: true, reduced: true }), false);
  assert.equal(schoolHasWork({ ...idle, roomOpen: true }), true, 'a live room animates');
  for (const k of ['tween', 'doorsMoving'] as const) assert.equal(schoolHasWork({ ...idle, [k]: true }), true, k);
  assert.equal(schoolHasWork({ ...idle, keys: 1 }), true);
  assert.equal(schoolHasWork({ ...idle, kicks: 1 }), true);
});

test('embed loop: a still embed has no work; motion of any kind keeps it going', () => {
  const idle = { needsFrame: false, tween: false, autoRotate: false, reduced: false, tilting: false, damping: false };
  assert.equal(embedHasWork(idle), false);
  assert.equal(embedHasWork({ ...idle, autoRotate: true, reduced: true }), false);
  for (const k of ['needsFrame', 'tween', 'autoRotate', 'tilting', 'damping'] as const) assert.equal(embedHasWork({ ...idle, [k]: true }), true, k);
});

test('viewport loop: only tweens and an undragged turntable re-arm', () => {
  const idle = { tween: false, camTween: false, turntable: false, dragging: false };
  assert.equal(viewportHasWork(idle), false);
  assert.equal(viewportHasWork({ ...idle, turntable: true, dragging: true }), false);
  for (const k of ['tween', 'camTween', 'turntable'] as const) assert.equal(viewportHasWork({ ...idle, [k]: true }), true, k);
});

test('viewport shortcuts: hovering alone (focus elsewhere) never owns the key', () => {
  const inside = { id: 'canvas' };
  const host = { contains: (n: unknown) => n === inside };
  assert.equal(viewportOwnsKey(host, inside as unknown as EventTarget), true);
  assert.equal(viewportOwnsKey(host, { id: 'body' } as unknown as EventTarget), false, 'a Backspace typed elsewhere must not delete');
  assert.equal(viewportOwnsKey(null, inside as unknown as EventTarget), false);
});

test('the three surfaces use the shared loop and visibility helpers (no hand-rolled rAF / observers)', () => {
  const src = (p: string) => readFileSync(join(import.meta.dirname, '..', 'src', p), 'utf8');
  const surfaces = ['modules/home/school/scene.ts', 'modules/design-hall/studio3d/embed.ts', 'lib/design/scene3d/Scene3DViewport.svelte'];
  for (const p of surfaces) {
    const s = src(p);
    assert.match(s, /demandLoop\(/, p);
    assert.match(s, /watchVisibility\(/, p);
    assert.doesNotMatch(s, /requestAnimationFrame\(/, `${p} re-arms rAF by hand`);
    assert.doesNotMatch(s, /new IntersectionObserver|addEventListener\('visibilitychange'/, `${p} hand-rolls visibility`);
  }
  assert.doesNotMatch(src('lib/design/scene3d/Scene3DViewport.svelte'), /window\.addEventListener\('keydown'/, 'shortcuts bind on the viewport, not the window');
  assert.match(src('modules/design-hall/studio3d/embed.ts'), /onContextLost\(canvas/);
});
