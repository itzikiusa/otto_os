// Classrooms 3D scene lifecycle (S14-01/S14-04/S14-05/S14-12): the scene
// itself needs WebGL, so the kick bookkeeping is driven directly and the
// WebGL lifecycle wiring is pinned statically (like the box-wiring test in
// classrooms.test.ts).
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { settleKicks } from '../src/modules/home/classrooms/kicks.ts';

const scene = readFileSync(join(import.meta.dirname, '..', 'src/modules/home/classrooms/scene.ts'), 'utf8');
const box = readFileSync(join(import.meta.dirname, '..', 'src/modules/home/boxes/ClassroomsBox.svelte'), 'utf8');

test('a refetch that drops a student mid-walk settles its kick-out (the toast fires)', async () => {
  const kicks = new Map<string, { resolve: () => void }>();
  let resolved = false;
  const done = new Promise<void>((resolve) => kicks.set('gone', { resolve }));
  void done.then(() => (resolved = true));
  let other = false;
  kicks.set('still', { resolve: () => (other = true) });
  assert.deepEqual(settleKicks(kicks, new Set(['still', 'x'])), ['gone']);
  await done;
  assert.equal(resolved, true);
  assert.equal(other, false, 'a student still in the model keeps walking');
  assert.deepEqual([...kicks.keys()], ['still']);
  assert.deepEqual(settleKicks(kicks, new Set(['still'])), [], 'idempotent');
});

test('scene.update() settles orphaned kicks before rebuilding', () => {
  const upd = scene.slice(scene.indexOf('    update(m) {'), scene.indexOf('    setColors(c) {'));
  assert.match(upd, /settleKicks\(kicks, ids\)/);
  assert.ok(upd.indexOf('settleKicks') < upd.indexOf('build()'), 'settle before build() drops the slots');
});

test('destroy() releases the GL context and resolves pending kicks', () => {
  const d = scene.slice(scene.indexOf('    destroy() {'));
  assert.match(d, /renderer\.forceContextLoss\(\)/);
  assert.match(d, /for \(const k of kicks\.values\(\)\) k\.resolve\(\)/);
  assert.match(d, /removeEventListener\('webglcontextlost'/);
});

test('the WebGL probe runs once and releases its context', () => {
  const p = scene.slice(scene.indexOf('export function webglAvailable'));
  assert.match(p, /if \(webglProbe !== null\) return webglProbe/);
  assert.match(p, /WEBGL_lose_context'\)\?\.loseContext\(\)/);
});

test('a lost context surfaces as the box error with Retry', () => {
  assert.match(scene, /addEventListener\('webglcontextlost', onLost\)/);
  assert.match(scene, /e\.preventDefault\(\)/);
  assert.match(box, /x\.onContextLost\(/);
  assert.match(box, /sceneError = 'The 3D view lost its graphics context/);
});

test('label projection reads the canvas rect once per frame', () => {
  assert.match(scene, /const rect = frameRect \?\? canvas\.getBoundingClientRect\(\)/);
  const f = scene.slice(scene.indexOf('  function frame(now: number)'), scene.indexOf('  const resize = '));
  assert.match(f, /frameRect = canvas\.getBoundingClientRect\(\);\s*try \{\s*frameFn\?\.\(\);\s*\} finally \{\s*frameRect = null;/);
});

test('3D keyboard path: both row buttons show the card; the card is not a tooltip', () => {
  assert.equal((box.match(/onfocus=\{\(\) => focusStudent\(s\.id\)\}/g) ?? []).length, 2);
  assert.doesNotMatch(box, /\n\s*role="tooltip"/);
  assert.match(box, /aria-describedby=\{tip\?\.id === s\.id \? tipId : undefined\}/);
});
