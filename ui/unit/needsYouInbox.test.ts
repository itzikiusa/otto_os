import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { needsYouLinkLabel } from '../src/modules/home/needsYou.ts';
import { AUTOMATE_TASKS_TABS } from '../src/lib/automateTasksTabs.ts';

test('the shared inbox link says how many things need you (S20-07)', () => {
  assert.equal(needsYouLinkLabel(0), 'Needs you inbox');
  assert.equal(needsYouLinkLabel(1), '1 needs you · Open inbox');
  assert.equal(needsYouLinkLabel(4), '4 need you · Open inbox');
});

test('every narrower needs-you surface links to the one canonical inbox', () => {
  for (const file of [
    '../src/modules/mcp/ApprovalsTab.svelte',
  ]) {
    const src = readFileSync(new URL(file, import.meta.url), 'utf8');
    assert.match(src, /<NeedsYouLink\b/, `${file} must render the shared NeedsYouLink`);
  }
});

test('the agent-to-agent transport is never labelled "Rooms" (the sidebar module owns that noun)', () => {
  const page = readFileSync(new URL('../src/modules/personal-agents/PersonalAgentsPage.svelte', import.meta.url), 'utf8');
  assert.doesNotMatch(page, />Agent rooms</);
  const labels = AUTOMATE_TASKS_TABS.map((t) => t.label);
  assert.ok(labels.includes('Agent channels'));
  assert.ok(!labels.some((l) => /\broom\b/i.test(l)));
  const sidebar = readFileSync(new URL('../src/lib/sidebar.ts', import.meta.url), 'utf8');
  const st = sidebar.slice(sidebar.indexOf("id: 'scheduled-tasks'"), sidebar.indexOf('},', sidebar.indexOf("id: 'scheduled-tasks'")));
  assert.doesNotMatch(st, /\broom\b/);
});
