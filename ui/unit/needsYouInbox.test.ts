import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { needsYouLinkLabel } from '../src/modules/home/needsYou.ts';

test('the shared inbox link says how many things need you (S20-07)', () => {
  assert.equal(needsYouLinkLabel(0), 'Needs you inbox');
  assert.equal(needsYouLinkLabel(1), '1 needs you · Open inbox');
  assert.equal(needsYouLinkLabel(4), '4 need you · Open inbox');
});

test('every narrower needs-you surface links to the one canonical inbox', () => {
  for (const file of [
    '../src/modules/assistant/NeedsYouRail.svelte',
    '../src/modules/agents/WorkQueue.svelte',
    '../src/modules/mission-control/MissionControlPage.svelte',
    '../src/modules/mcp/ApprovalsTab.svelte',
  ]) {
    const src = readFileSync(new URL(file, import.meta.url), 'utf8');
    assert.match(src, /<NeedsYouLink\b/, `${file} must render the shared NeedsYouLink`);
  }
});

test('the agent-to-agent transport is never labelled "Rooms" (the sidebar module owns that noun)', () => {
  const page = readFileSync(new URL('../src/modules/personal-agents/PersonalAgentsPage.svelte', import.meta.url), 'utf8');
  assert.doesNotMatch(page, />Agent rooms</);
  assert.match(page, />Agent channels</);
  const sidebar = readFileSync(new URL('../src/lib/sidebar.ts', import.meta.url), 'utf8');
  const pa = sidebar.split('\n').find((l) => l.includes("id: 'personal-agents'")) ?? '';
  assert.doesNotMatch(pa, /\broom\b/);
});
