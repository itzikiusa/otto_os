// Product → story → Design tab (product/storyDesigns.ts): the routes a story's
// designs open (the artifact view reads `from=story/<id>` for its way back)
// and the Overview's count line. Pure functions, run through the source
// harness with the real lib/plural. The
// components themselves are driven by desktop-story-designs.spec.ts and
// story-designs-mobile.spec.ts.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { loadSource } from './sourceHarness.ts';
import * as pluralMod from '../src/lib/plural.ts';

const { designCountLabel, storyArtifactRoute, storyHallRoute, storyLinkRoute } = loadSource(
  new URL('../src/modules/product/storyDesigns.ts', import.meta.url),
  { '../../lib/plural': pluralMod },
);

test('a design opened from a story carries ?from=story/<id>, with an optional Otto tab', () => {
  assert.equal(storyArtifactRoute('A1', 'S1'), 'design/a/A1?from=story/S1');
  assert.equal(storyArtifactRoute('A1', 'S1', 'otto'), 'design/a/A1/otto?from=story/S1');
  // The artifact view's reader (`/[?&]from=story\/([^&/]+)/` on the hash) finds the story.
  const m = /[?&]from=story\/([^&/]+)/.exec(`#/${storyArtifactRoute('A1', 'S1', 'otto')}`);
  assert.equal(m?.[1], 'S1');
});

test('only a design card link is rewritten; other grid links keep their target', () => {
  assert.equal(storyLinkRoute('#/design/a/A%201', 'S1'), 'design/a/A%201?from=story/S1');
  assert.equal(storyLinkRoute('#/design/story/S2', 'S1'), null);
  assert.equal(storyLinkRoute('#/design/p/P1', 'S1'), null);
  assert.equal(storyLinkRoute('#/design/a/A1/otto', 'S1'), null);
  assert.equal(storyLinkRoute('#/design/a/%E0%A4', 'S1'), null, 'a malformed escape is left alone');
  assert.equal(storyLinkRoute(null, 'S1'), null);
});

test('the story page of the Hall and the Overview count line', () => {
  assert.equal(storyHallRoute('S 1'), 'design/story/S%201');
  assert.equal(designCountLabel(0), 'No designs yet');
  assert.equal(designCountLabel(1), '1 design');
  assert.equal(designCountLabel(3), '3 designs');
  assert.equal(designCountLabel(100, true), '100+ designs');
});

const src = (p: string) => readFileSync(new URL(`../src/${p}`, import.meta.url), 'utf8');

test('#/canvas forwards into Design Hall; the Canvas page is kept, unmounted', () => {
  const pages = src('shell/pages.svelte.ts');
  assert.match(pages, /canvas: \(\) => import\('\.\.\/modules\/canvas\/CanvasRedirect\.svelte'\)/);
  assert.doesNotMatch(pages, /import\('\.\.\/modules\/canvas\/CanvasPage\.svelte'\)/);
  // The one door other modules use: the scene's Hall mirror, never the old page.
  const nav = src('modules/design-hall/nav.ts');
  assert.match(nav, /source_kind: 'canvas_scene', source_id: sceneId/);
  assert.doesNotMatch(nav, /router\.go\('canvas'\)|pendingOpenId/);
});
