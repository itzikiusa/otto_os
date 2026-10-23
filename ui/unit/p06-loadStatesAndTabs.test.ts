import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';

// Iteration-6 P06 source pins: failed loads render through LoadState (never an
// EmptyState that reads as "nothing here"), repo tabs reorder without a mouse,
// and git/agent copy says "couldn’t".

const src = (p: string) => readFileSync(new URL(`../src/modules/${p}`, import.meta.url), 'utf8');

test('a failed agents load is not "Agent not found" (ux-design-6-04)', () => {
  const s = src('personal-agents/AgentPage.svelte');
  const err = s.indexOf('{:else if personalAgents.agentsError}');
  const notFound = s.indexOf('title="Agent not found"');
  assert.ok(err > 0 && err < notFound, 'the error branch comes before "Agent not found"');
  assert.match(s.slice(err, notFound), /<LoadState what="this agent"[^>]*onretry=/);
});

test('PR list and Mission Control load errors use LoadState (ux-design-6-02)', () => {
  assert.match(src('git/PrList.svelte'), /<LoadState what="pull requests"/);
  assert.doesNotMatch(src('git/PrList.svelte'), /title=\{errorTitle\}/);
  const mc = src('mission-control/MissionControlPage.svelte');
  assert.match(mc, /<LoadState what="Mission Control"/);
  assert.doesNotMatch(mc, /title="Couldn’t load Mission Control"/);
});

test('repo tabs move with ⌥⇧←/→ and a Move left/right menu (a11y-6-04)', () => {
  const s = src('git/GitTabs.svelte');
  assert.match(s, /e\.altKey && e\.shiftKey[^\n]*ArrowLeft/);
  assert.match(s, /label: 'Move left', hint: '⌥⇧←'/);
  assert.match(s, /label: 'Move right', hint: '⌥⇧→'/);
  assert.match(s, /oncontextmenu=\{\(e\) => tabMenu\(e, r\.id\)\}/);
});

test('no "could not" / "Cannot open" / "failed to" copy in P06 modules (ux-design-6-08)', () => {
  for (const p of ['git/GraphView.svelte', 'git/pushFlow.ts', 'git/PrList.svelte', 'git/FocusView.svelte', 'panels/FileTree.svelte', 'personal-agents/AgentEditSheet.svelte']) {
    assert.doesNotMatch(src(p), /could not be|Cannot open|failed to load PRs|failed to list/, p);
  }
});
