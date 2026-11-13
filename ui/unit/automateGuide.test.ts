import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { runInNewContext } from 'node:vm';
import ts from 'typescript';
import * as sidebar from '../src/lib/sidebar.ts';

const src = (rel: string) => readFileSync(new URL(rel, import.meta.url), 'utf8');

/** Execute the chooser's `<script module>` (the data + the pure filter) with
 *  the REAL sidebar gate. Objects come from another vm realm: compare via JSON. */
function guideModule(): Record<string, any> {
  const guide = src('../src/lib/components/AutomateGuide.svelte');
  const body = /<script lang="ts" module>([\s\S]*?)<\/script>/.exec(guide)?.[1];
  assert.ok(body, 'AutomateGuide keeps its data in a module script');
  const js = ts.transpileModule(body, { compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2022 } }).outputText;
  const exports: Record<string, any> = {};
  runInNewContext(js, {
    exports,
    require: (name: string) => {
      if (name === '../sidebar') return sidebar;
      throw new Error(`unexpected import ${name}`);
    },
  });
  return exports;
}
const plain = <T>(x: T): T => JSON.parse(JSON.stringify(x));
const ALL = ['scheduled-tasks', 'workflows', 'loops', 'personal-agents'];

test('the chooser has four rows that lead with the job and end with the place', () => {
  const g = guideModule();
  const rows = plain(g.AUTOMATE_GUIDE) as { id: string; route: string; label: string; job: string }[];
  assert.deepEqual(rows.map((r) => r.id), ALL);
  assert.deepEqual(rows.map((r) => r.label), ['Scheduled tasks', 'Workflows', 'Goal Loops', 'Personal Agents']);
  assert.match(rows[0].job, /one prompt or command on a schedule/i);
  assert.match(rows[1].job, /several steps/i);
  assert.match(rows[2].job, /until a goal is met or the budget runs out/i);
  assert.match(rows[3].job, /named assistant with memory/i);
  for (const r of rows) assert.equal(r.route, r.id, `${r.id} opens its own place`);
  assert.match(g.AUTOMATE_GUIDE_FOOTER, /^Every morning\? One prompt → a Scheduled task\. Several steps → a Workflow with a Schedule trigger\.$/);
  // Every route lands on a real sidebar module (personal-agents is the hub home).
  const side = src('../src/lib/sidebar.ts');
  for (const r of rows) assert.match(side, new RegExp(`'${r.route}'`), `${r.route} must route to a known sidebar module`);
});

test('every row passes the sidebar role check for a person who can open everything', () => {
  const g = guideModule();
  for (const r of plain(g.AUTOMATE_GUIDE) as { route: string }[]) {
    assert.equal(sidebar.routeAllowed(r.route, () => true), true, r.route);
  }
  for (const current of ALL) {
    assert.deepEqual(plain(g.automateGuideEntries(current, () => true)).map((r: any) => r.id), ALL);
  }
});

test('rows the person cannot open are hidden; the current page always stays (S20-307)', () => {
  const g = guideModule();
  const none = () => false;
  // Only the ungated place (Goal Loops) survives with no grants…
  assert.deepEqual(plain(g.automateGuideEntries('loops', none)).map((r: any) => r.id), ['loops']);
  // …and the current page is kept even when its feature reads as off.
  assert.deepEqual(plain(g.automateGuideEntries('workflows', none)).map((r: any) => r.id), ['workflows', 'loops']);
  // The scheduled_tasks grant opens both Scheduled tasks and Personal Agents.
  assert.deepEqual(
    plain(g.automateGuideEntries('loops', (f: string) => f === 'scheduled_tasks')).map((r: any) => r.id),
    ['scheduled-tasks', 'loops', 'personal-agents'],
  );
});

test('`current` marks exactly one row as this page, for each of the four pages', () => {
  const g = guideModule();
  for (const current of ALL) {
    const here = plain(g.automateGuideEntries(current, () => true)).filter((r: any) => r.here).map((r: any) => r.id);
    assert.deepEqual(here, [current]);
  }
});

test('every Automate page passes its own `current`, opens the chooser on its empty state and keeps the ⓘ', () => {
  for (const [file, current] of [
    ['scheduled-tasks/ScheduledTasksPage', 'scheduled-tasks'],
    ['personal-agents/PersonalAgentsPage', 'personal-agents'],
    ['workflows/WorkflowsPage', 'workflows'],
    ['loops/LoopsPage', 'loops'],
  ]) {
    const page = src(`../src/modules/${file}.svelte`);
    assert.match(page, new RegExp(`<AutomateGuideButton current="${current}" />`), `${file} header must carry the chooser`);
    assert.match(page, new RegExp(`<AutomateGuide current="${current}" open />`), `${file} empty state shows the chooser open`);
    assert.doesNotMatch(page, new RegExp(`<AutomateGuide current="(?!${current}")`), `${file} passes no other current`);
  }
});
