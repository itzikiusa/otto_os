// accessibility-responsive-6-05 / 6-08 / 6-09 (P10): controls that unmount
// themselves move focus to their successor; viewport shortcuts need focus.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { isTypingTarget, viewportOwnsKey } from '../src/lib/design/scene3d/viewportLoop.ts';

const src = (p: string) => readFileSync(join(import.meta.dirname, '..', 'src', p), 'utf8');

test('site Inspector: every selection change it makes goes through the focus-moving go()', () => {
  const s = src('modules/design-hall/site/Inspector.svelte');
  const calls = [...s.matchAll(/\bonselect\(/g)].length;
  assert.equal(calls, 1, 'only go() calls onselect');
  assert.match(s, /void go\(section\.id, null, block!\.id\)/, 'back lands on the row of the block just left');
  assert.equal([...s.matchAll(/data-view-head/g)].length >= 3, true, 'block, section and page views each have a focus target');
  assert.match(s, /data-item=\{b\.id\}/);
});

test('site Inspector link rows: stable keys, per-row names, focus after removal', () => {
  const s = src('modules/design-hall/site/LinkListField.svelte');
  assert.match(s, /\{#each keyed as \{ l, key \}, i \(key\)\}/);
  assert.match(s, /aria-label=\{`Remove link \$\{rowName\(l, i\)\}`\}/);
  assert.match(s, /querySelectorAll<HTMLElement>\('\[data-remove-row\]'\)/);
  assert.doesNotMatch(src('modules/design-hall/site/Inspector.svelte'), /aria-label="Remove link"/);
});

test('School breadcrumbs move focus to the stage before the crumb unmounts', () => {
  const s = src('modules/home/boxes/ClassroomsBox.svelte');
  assert.match(s, /const upTo = [\s\S]*?stage\?\.focus\(\{ preventScroll: true \}\)/);
  assert.match(s, /go: view\.kind === 'corridor' \? undefined : upTo\(/);
});

test('3D viewport: typing in a field inside it is never a shortcut', () => {
  const field = { tagName: 'INPUT' } as unknown as EventTarget;
  const host = { contains: () => true };
  assert.equal(isTypingTarget(field), true);
  assert.equal(viewportOwnsKey(host, field), false);
  assert.equal(viewportOwnsKey(host, { tagName: 'CANVAS' } as unknown as EventTarget), true);
});
