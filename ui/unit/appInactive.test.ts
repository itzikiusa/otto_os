// Infinite animations pause while nobody is looking (lib/appInactive.ts);
// one-shot entrances (a toast, a modal) always play to the end.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { loadSource } from './sourceHarness.ts';

type Anim = { iterations: number; playState: string; effect: unknown; pause(): void; play(): void };
const anim = (iterations: number): Anim => {
  const a: Anim = {
    iterations, playState: 'running',
    effect: { getComputedTiming: () => ({ iterations }) },
    pause() { a.playState = 'paused'; },
    play() { a.playState = 'running'; },
  };
  return a;
};

function fixture(opts: { hidden?: boolean; embedded?: boolean; nativePane?: boolean } = {}) {
  const on = new Map<string, (e?: unknown) => void>();
  const timers: (() => void)[] = [];
  const live: Anim[] = [];
  const doc = {
    hidden: !!opts.hidden, focus: !opts.hidden,
    hasFocus() { return this.focus; },
    getAnimations: () => live,
    addEventListener: (t: string, fn: (e?: unknown) => void) => on.set('doc:' + t, fn),
    removeEventListener: (t: string) => on.delete('doc:' + t),
  };
  const win: Record<string, unknown> = { addEventListener: (t: string, fn: () => void) => on.set('win:' + t, fn) };
  win.parent = opts.embedded ? {} : win;
  const m = loadSource(new URL('../src/lib/appInactive.ts', import.meta.url), {}, {
    setTimeout: (fn: () => void) => (timers.push(fn), timers.length),
    clearTimeout: () => void timers.splice(0),
  });
  if (opts.nativePane) m.installAppInactive(doc, win, true);
  else m.installAppInactive(doc, win);
  return { doc, on, timers, live };
}

test('a toast raised while unfocused plays; the pulses pause; focus resumes them', () => {
  const f = fixture();
  const pulse = anim(Infinity);
  f.live.push(pulse);
  f.doc.focus = false; f.on.get('win:blur')!();
  assert.equal(pulse.playState, 'running', 'not during the grace period');
  f.timers.at(-1)!();
  assert.equal(pulse.playState, 'paused');
  // Raised while away: the toast's entrance runs, a new spinner pauses.
  const toast = anim(1);
  const spinner = anim(Infinity);
  f.live.push(toast, spinner);
  f.on.get('doc:animationstart')!({ target: { getAnimations: () => [toast] } });
  f.on.get('doc:animationstart')!({ target: { getAnimations: () => [spinner] } });
  assert.equal(toast.playState, 'running', 'one-shot entrance never paused');
  assert.equal(spinner.playState, 'paused');
  f.doc.focus = true; f.on.get('win:focus')!();
  assert.equal(pulse.playState, 'running');
  assert.equal(spinner.playState, 'running');
});

test('the grace timer re-checks focus: back within 5 s, nothing pauses', () => {
  const f = fixture();
  const pulse = anim(Infinity);
  f.live.push(pulse);
  f.doc.focus = false; f.on.get('win:blur')!();
  const fire = f.timers.at(-1)!;
  f.doc.focus = true; // focus returned without a focus event reaching us
  fire();
  assert.equal(pulse.playState, 'running');
});

test('a window created hidden starts paused; an embedded pane pauses only while hidden', () => {
  const hidden = fixture({ hidden: true });
  assert.ok(hidden.on.has('doc:animationstart'), 'paused from the start');
  const pane = fixture({ embedded: true });
  pane.doc.focus = false; pane.on.get('win:blur')!();
  assert.equal(pane.timers.length, 0);
  assert.ok(!pane.on.has('doc:animationstart'));
});

test('no stylesheet rule pauses every animation, and main.ts installs the pause', () => {
  assert.doesNotMatch(readFileSync(new URL('../src/app.css', import.meta.url), 'utf8'), /app-inactive/);
  assert.match(readFileSync(new URL('../src/main.ts', import.meta.url), 'utf8'), /installAppInactive\(document, window, isEmbedded\);/);
});

test('the native side pane (window.parent is itself) pauses only while hidden, never on blur', () => {
  const pane = fixture({ nativePane: true });
  pane.doc.focus = false; pane.on.get('win:blur')!();
  assert.equal(pane.timers.length, 0);
  assert.ok(!pane.on.has('doc:animationstart'));
});

test('an animation that left the page while away is neither kept nor resumed', () => {
  const f = fixture();
  const kept = anim(Infinity);
  const removed = anim(Infinity);
  f.live.push(kept, removed);
  f.doc.hidden = true; f.doc.focus = false; f.on.get('doc:visibilitychange')!();
  assert.equal(removed.playState, 'paused');
  f.live.splice(f.live.indexOf(removed), 1); // its dot unmounted meanwhile
  f.doc.hidden = false; f.doc.focus = true; f.on.get('win:focus')!();
  assert.equal(kept.playState, 'running');
  assert.equal(removed.playState, 'paused', 'a detached animation is not revived');
  assert.match(readFileSync(new URL('../src/lib/appInactive.ts', import.meta.url), 'utf8'), /paused = new WeakSet<Animation>\(\)/, 'no strong list of paused animations');
});
