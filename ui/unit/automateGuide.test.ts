import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';

const src = (rel: string) => readFileSync(new URL(rel, import.meta.url), 'utf8');

test('the Automate chooser lists the overlapping automations that live outside Automate (S20-18)', () => {
  const guide = src('../src/lib/components/AutomateGuide.svelte');
  assert.match(guide, /id: 'assistant-tasks', route: 'assistant\/tasks'/);
  assert.match(guide, /id: 'run-with-otto', route: 'run-with-otto'/);
  // Every route must land on a real sidebar module.
  const sidebar = src('../src/lib/sidebar.ts');
  for (const m of guide.matchAll(/route: '([^']+)'/g)) {
    const mod = m[1].split('/')[0];
    assert.match(sidebar, new RegExp(`id: '${mod}'`), `${m[1]} must route to a sidebar module`);
  }
});

test('every Automate page offers the chooser beyond its empty state', () => {
  for (const [file, current] of [
    ['swarm/SwarmPage', 'swarm'],
    ['scheduled-tasks/ScheduledTasksPage', 'scheduled-tasks'],
    ['personal-agents/PersonalAgentsPage', 'personal-agents'],
    ['workflows/WorkflowsPage', 'workflows'],
    ['loops/LoopsPage', 'loops'],
  ]) {
    const page = src(`../src/modules/${file}.svelte`);
    assert.match(page, new RegExp(`<AutomateGuideButton current="${current}" />`), `${file} header must carry the chooser`);
  }
});
