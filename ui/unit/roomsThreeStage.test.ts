/// <reference lib="dom" />
// architecture-5-05: the shared three.js lifecycle (lib/three/stage.ts) the
// classroom, the room games and the studio embed use.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { cappedPixelRatio, lossBudget, onContextLost, releaseRenderer } from '../src/lib/three/stage.ts';

test('pixel ratio is capped and a bogus ratio counts as 1', () => {
  assert.equal(cappedPixelRatio(1.75, 3), 1.75);
  assert.equal(cappedPixelRatio(2, 1), 1);
  assert.equal(cappedPixelRatio(2, Number.NaN), 1);
  assert.equal(cappedPixelRatio(2, 0), 1);
});

test('a lost context is prevented (restorable) and reported; unsubscribe stops it', () => {
  const target = new EventTarget() as unknown as HTMLCanvasElement;
  let lost = 0;
  const off = onContextLost(target, () => lost++);
  const e = new Event('webglcontextlost', { cancelable: true });
  target.dispatchEvent(e);
  assert.equal(lost, 1);
  assert.equal(e.defaultPrevented, true);
  off();
  target.dispatchEvent(new Event('webglcontextlost', { cancelable: true }));
  assert.equal(lost, 1);
});

test('releasing a renderer disposes, forces the context loss and removes the canvas, in that order', () => {
  const calls: string[] = [];
  releaseRenderer({ dispose: () => calls.push('dispose'), forceContextLoss: () => calls.push('lose'), domElement: { remove: () => calls.push('remove') } as unknown as HTMLCanvasElement });
  assert.deepEqual(calls, ['dispose', 'lose', 'remove']);
});

test('the remount budget allows two losses a minute, then asks for Retry', () => {
  let t = 0;
  const budget = lossBudget(2, 60_000, () => t);
  assert.equal(budget.allow(), true);
  t = 1000; assert.equal(budget.allow(), true);
  t = 2000; assert.equal(budget.allow(), false);
  t = 70_000; assert.equal(budget.allow(), true);
});

test('every three.js surface goes through the shared stage helpers', () => {
  const src = (p: string) => readFileSync(join(import.meta.dirname, '..', 'src', p), 'utf8');
  for (const p of ['modules/rooms/games/common/view.ts', 'modules/home/school/scene.ts', 'modules/design-hall/studio3d/embed.ts']) {
    assert.match(src(p), /releaseRenderer\(/, p);
    assert.doesNotMatch(src(p), /forceContextLoss\(\)/, `${p} hand-rolls the teardown`);
  }
  assert.match(src('modules/rooms/games/common/view.ts'), /onContextLost\(this\.canvas/);
  for (const p of ['modules/rooms/games/kart/KartGame.svelte', 'modules/rooms/games/shooter/ShooterGame.svelte']) assert.match(src(p), /created\.view\.onContextLost\(/, p);
});
