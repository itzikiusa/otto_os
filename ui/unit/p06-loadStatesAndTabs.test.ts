import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { componentFunctions } from './componentFunctions.ts';

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

// a11y-6-04, behaviour (test-quality-6w1-06): run GitTabs' real key handler
// and mover over a fake store and stub tab elements; assert the strip order
// and which tab ends up focused.
function gitTabs(order: string[], dir: 'ltr' | 'rtl' = 'ltr') {
  const focused: string[] = [];
  const opened: string[] = [];
  const rovingKeys: string[] = [];
  const git = {
    openRepoIds: [...order],
    reorderRepoTab(id: string, to: number) {
      const ids = this.openRepoIds.filter((x) => x !== id);
      ids.splice(to, 0, id);
      this.openRepoIds = ids;
    },
    closeRepoTab() {},
  };
  const listEl = {
    querySelector: (sel: string) => {
      const id = /data-repo-id="([^"]+)"/.exec(sel)?.[1];
      return id ? { focus: () => focused.push(id) } : null;
    },
  };
  const fns = componentFunctions(new URL('../src/modules/git/GitTabs.svelte', import.meta.url), ['onRepoTabKey', 'moveTab'], {
    git,
    listEl,
    openRepos: order.map((id) => ({ id })),
    onopen: (id: string) => opened.push(id),
    onTabKey: (e: { key: string }) => rovingKeys.push(e.key),
    tick: () => Promise.resolve(),
    queueMicrotask,
    getComputedStyle: () => ({ direction: dir }),
    CSS: { escape: (s: string) => s },
  });
  const key = async (id: string, k: string, mods: { altKey?: boolean; shiftKey?: boolean; metaKey?: boolean } = {}) => {
    let prevented = false;
    fns.onRepoTabKey({ key: k, altKey: false, shiftKey: false, metaKey: false, ctrlKey: false, ...mods, currentTarget: {}, preventDefault: () => { prevented = true; } }, id);
    await new Promise((r) => setTimeout(r, 0));
    return prevented;
  };
  return { git, focused, opened, rovingKeys, key };
}

test('⌥⇧→ / ⌥⇧← move the focused repo tab one place and keep focus on it (a11y-6-04)', async () => {
  const t = gitTabs(['a', 'b', 'c']);
  assert.equal(await t.key('a', 'ArrowRight', { altKey: true, shiftKey: true }), true);
  assert.deepEqual([...t.git.openRepoIds], ['b', 'a', 'c']);
  assert.deepEqual(t.focused, ['a'], 'focus follows the moved tab');
  await t.key('a', 'ArrowLeft', { altKey: true, shiftKey: true });
  assert.deepEqual([...t.git.openRepoIds], ['a', 'b', 'c']);
  // At the edge nothing moves (and nothing is re-focused).
  await t.key('a', 'ArrowLeft', { altKey: true, shiftKey: true });
  assert.deepEqual([...t.git.openRepoIds], ['a', 'b', 'c']);
  assert.deepEqual(t.focused, ['a', 'a']);
  // Plain arrows stay the roving-tabindex move (no reorder); ⌘⌥⇧ is not ours.
  await t.key('b', 'ArrowRight');
  await t.key('b', 'ArrowRight', { altKey: true, shiftKey: true, metaKey: true });
  assert.deepEqual(t.rovingKeys, ['ArrowRight', 'ArrowRight']);
  assert.deepEqual([...t.git.openRepoIds], ['a', 'b', 'c']);
});

test('in RTL ⌥⇧→ moves the tab toward the start of the strip', async () => {
  const t = gitTabs(['a', 'b', 'c'], 'rtl');
  await t.key('b', 'ArrowRight', { altKey: true, shiftKey: true });
  assert.deepEqual([...t.git.openRepoIds], ['b', 'a', 'c']);
});

test('the tab menu offers Move left/right with the shortcut hints (structure)', () => {
  const s = src('git/GitTabs.svelte');
  assert.match(s, /label: 'Move left', hint: '⌥⇧←'/);
  assert.match(s, /label: 'Move right', hint: '⌥⇧→'/);
  assert.match(s, /oncontextmenu=\{\(e\) => tabMenu\(e, r\.id\)\}/);
});

test('no "could not" / "Cannot open" / "failed to" copy in P06 modules (ux-design-6-08)', () => {
  for (const p of ['git/GraphView.svelte', 'git/pushFlow.ts', 'git/PrList.svelte', 'git/FocusView.svelte', 'panels/FileTree.svelte', 'personal-agents/AgentEditSheet.svelte']) {
    assert.doesNotMatch(src(p), /could not be|Cannot open|failed to load PRs|failed to list/, p);
  }
});
