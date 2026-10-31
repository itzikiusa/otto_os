import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';

const src = (rel: string) => readFileSync(new URL(rel, import.meta.url), 'utf8');

test('the Automate chooser lists the core Automate surfaces and maps every route to a sidebar module', () => {
  const guide = src('../src/lib/components/AutomateGuide.svelte');
  assert.match(guide, /id: 'workflows', route: 'workflows'/);
  assert.match(guide, /id: 'loops', route: 'loops'/);
  assert.match(guide, /id: 'scheduled-tasks', route: 'scheduled-tasks'/);
  // Every route must land on a real sidebar module (or redirect handled by sidebar).
  const sidebar = src('../src/lib/sidebar.ts');
  for (const m of guide.matchAll(/route: '([^']+)'/g)) {
    const mod = m[1].split('/')[0];
    assert.match(sidebar, new RegExp(`'${mod}'`), `${m[1]} must route to a known sidebar module`);
  }
});

test('every Automate page offers the chooser beyond its empty state', () => {
  for (const [file, current] of [
    ['scheduled-tasks/ScheduledTasksPage', 'scheduled-tasks'],
    ['personal-agents/PersonalAgentsPage', 'scheduled-tasks'],
    ['workflows/WorkflowsPage', 'workflows'],
    ['loops/LoopsPage', 'loops'],
  ]) {
    const page = src(`../src/modules/${file}.svelte`);
    assert.match(page, new RegExp(`<AutomateGuideButton current="${current}" />`), `${file} header must carry the chooser`);
  }
});

test('the chooser hides modules the user cannot open, using the sidebar gate (S20-307)', async () => {
  const { routeAllowed } = await import('../src/lib/sidebar.ts');
  const none = () => false;
  // Gated modules drop out when their feature is off…
  assert.equal(routeAllowed('personal-agents', none), false);
  assert.equal(routeAllowed('scheduled-tasks', none), false);
  assert.equal(routeAllowed('workflows', none), false);
  // …ungated ones (Goal Loops) always stay.
  assert.equal(routeAllowed('loops', none), true);
  // The matching feature re-enables exactly that entry.
  assert.equal(routeAllowed('personal-agents', (f) => f === 'scheduled_tasks'), true);
  assert.equal(routeAllowed('workflows', (f) => f === 'scheduled_tasks'), false);
  // The component actually filters through it (current page always kept).
  const guide = src('../src/lib/components/AutomateGuide.svelte');
  assert.match(guide, /routeAllowed\(m\.route, \(f\) => auth\.can\(f, 'view'\)\)/);
  assert.match(guide, /\{#each entries as m/);
});
