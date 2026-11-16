// Git bug hunt 2026-11-16 wave 2 (G2): branch-menu guards and the Create-PR
// target default.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { loadSource } from './sourceHarness.ts';

const refTracking = () => loadSource(new URL('../src/modules/git/refTracking.ts', import.meta.url), {});

function branchMenuLabels(opts: { canOpenPr: boolean; worktreeBranch?: string }, name: string): string[] {
  const menus: any[][] = [];
  const mod = loadSource(new URL('../src/modules/git/graph/graphMenus.ts', import.meta.url), {
    '../../../lib/contextmenu.svelte': { ctxMenu: { show: (_e: any, items: any[]) => menus.push(items) } },
    '../../../lib/stores/ui.svelte': { ui: {} },
    '../refTracking': refTracking(),
    '../../../lib/diff/gitBridge.svelte': { gitBridge: {} },
  });
  const wt = new Map(opts.worktreeBranch ? [[opts.worktreeBranch, { path: '/wt', branch: opts.worktreeBranch }]] : []);
  const host = {
    repoId: 'r1',
    onmergerequest: undefined,
    refs: null,
    selectedCommit: null,
    worktreeByBranch: wt,
    worktreeBranches: new Set(wt.keys()),
    refKnowledge: { remoteNames: new Set(), localNames: new Set(['main', name]), currentBranch: 'main' },
    baseBranch: 'main',
    canOpenPr: opts.canOpenPr,
    openCreatePr: () => {},
    chipsFor: () => [],
    setCleanupBase: async () => {},
  };
  const actions = new Proxy({}, { get: () => () => {} });
  mod.createGraphMenus(host, actions).branchMenu({}, { name, is_current: false, upstream: null, remote: false });
  return menus[0].map((i: any) => i.label).filter(Boolean);
}

test('a branch another worktree has checked out offers no Delete (git refuses it)', () => {
  const held = branchMenuLabels({ canOpenPr: true, worktreeBranch: 'feat' }, 'feat');
  assert.ok(!held.some((l) => l.startsWith('Delete feat')), held.join(' | '));
  const free = branchMenuLabels({ canOpenPr: true }, 'feat');
  assert.ok(free.includes('Delete feat…'), free.join(' | '));
});

test('no Create pull request on a repo whose remote is not a supported forge', () => {
  assert.ok(!branchMenuLabels({ canOpenPr: false }, 'feat').some((l) => l.startsWith('Create pull request')));
  assert.ok(branchMenuLabels({ canOpenPr: true }, 'feat').includes('Create pull request from feat…'));
});

test('Create PR target: never the source, and a remote-only base can be picked', () => {
  const { prTargetNames, defaultPrTarget } = refTracking();
  const names = prTargetNames(['develop', 'feature/x'], ['origin/HEAD', 'origin/develop', 'origin/master']);
  assert.deepEqual([...names], ['develop', 'feature/x', 'master']);
  assert.equal(defaultPrTarget(names, 'develop'), 'master');
  assert.equal(defaultPrTarget(names, 'feature/x'), 'develop');
  assert.equal(defaultPrTarget(['solo'], 'solo'), '');
});

test('canOpenPrs: supported forge yes; unrecognized or no remote no', () => {
  const { canOpenPrs, needResolution } = loadSource(new URL('../src/lib/api/git.ts', import.meta.url), {
    './client': { api: {} },
    '../plural': { plural: (n: number, w: string) => `${n} ${w}${n === 1 ? '' : 's'}` },
  });
  assert.equal(canOpenPrs({ forge: 'github', remote_url: 'git@github.com:a/b.git' }), true);
  assert.equal(canOpenPrs({ forge: 'unrecognized', remote_url: '/tmp/bare.git' }), false);
  assert.equal(canOpenPrs({ forge: null, remote_url: null }), false);
  assert.equal(needResolution(1), '1 file needs resolution');
  assert.equal(needResolution(2), '2 files need resolution');
});
