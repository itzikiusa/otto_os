import { test, expect, type Page, type Request } from '@playwright/test';
import { execFileSync } from 'node:child_process';
import { mkdtempSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { apiCtx, seedWorkspace } from './seed';
import { openPage } from './helpers';

// Opening a repository on the Git page (perf G6): what the daemon is asked
// for in the first seconds. The repos are local clones of THIS repository
// (thousands of commits, a real refs list), so the numbers are the ones the
// owner pays, not a three-commit fixture's. Both open on History: a clean
// clone on its checked-out commit, a dirty one on its WIP row.
//
// Budget: 7 requests for either — the overview, the opening fetch, the first
// log page (asked beside the overview, not after it), what the landing shows
// (a dirty clone's first WIP file diff, a clean one's HEAD commit summary and,
// for a small commit, its patch),
// the pull mode, and the cheap /refs check a fetch's refs bump asks for (the
// only way the graph learns what the fetch brought). The overview carries
// refs, stashes, worktrees and submodules: none is read again. The first log
// page asks for at most 200 commits (more load on scroll).

const CLEAN = 'e2e-open-perf-clean';
const DIRTY = 'e2e-open-perf-dirty';

// Every clone's temp root, removed in afterAll (pass or fail).
const roots: string[] = [];

test.beforeAll(async () => {
  const src = resolve(process.cwd(), '..');
  const { ctx, base } = await apiCtx();
  const wsId = await seedWorkspace(ctx, base);
  for (const name of [CLEAN, DIRTY]) {
    const root = mkdtempSync(join(tmpdir(), 'otto-e2e-open-perf-'));
    roots.push(root);
    const dir = join(root, 'r');
    // --shared borrows this repo's object store: the same history and refs at a
    // few MB (a --no-hardlinks copy left 1.2 GB per run in $TMPDIR).
    execFileSync('git', ['clone', '-q', '--shared', src, dir], { stdio: 'ignore' });
    if (name === DIRTY) writeFileSync(join(dir, 'PERF_PROBE.txt'), 'probe\n');
    const r = await ctx.post(`${base}/api/v1/workspaces/${wsId}/repos`, { data: { path: dir, name } });
    if (!r.ok()) throw new Error(`repo seed failed: ${r.status()} ${await r.text()}`);
  }
  await ctx.dispose();
});

test.afterAll(() => {
  for (const root of roots.splice(0)) rmSync(root, { recursive: true, force: true });
});

async function openAndCount(page: Page, name: string): Promise<string[]> {
  await openPage(page, 'git');
  const seen: string[] = [];
  const onReq = (r: Request): void => {
    const u = new URL(r.url());
    if (u.pathname.startsWith('/api/v1/repos/')) seen.push(`${r.method()} ${u.pathname.replace(/\/api\/v1\/repos\/[^/]+/, '')}${u.search}`);
  };
  // Open it from the + picker (the same path a person takes).
  await page.locator('.git-tab-new').click();
  const menu = page.locator('.ctx-menu');
  await menu.locator('.ctx-search-input').fill(name);
  page.on('request', onReq);
  await menu.getByRole('menuitem', { name }).first().click();
  await page.waitForTimeout(5_000); // ui-guards: allow — a fixed window counts everything the open fires
  page.off('request', onReq);
  // eslint-disable-next-line no-console
  console.log(`[open-perf] ${name}: ${seen.length} repo requests:\n  ${seen.join('\n  ')}`);
  return seen;
}

for (const name of [CLEAN, DIRTY]) {
  const budget = 7;
  test(`opening a repository (${name === CLEAN ? 'clean → History, HEAD' : 'dirty → History, WIP'}): ≤ ${budget} requests, first log page ≤ 200`, async ({ page }, info) => {
    test.skip(info.project.name !== 'desktop-browser', 'request counts are engine-independent');
    const seen = await openAndCount(page, name);
    const limits = seen.map((s) => /\/log\?.*limit=(\d+)/.exec(s)?.[1]).filter(Boolean).map(Number);
    expect(Math.max(0, ...limits), 'first log page size').toBeLessThanOrEqual(200);
    expect(seen.filter((s) => /\/(stashes|worktrees|submodules)(\?|$)/.test(s)), 'the overview carries the side lists').toEqual([]);
    expect(seen.length, seen.join('\n')).toBeLessThanOrEqual(budget);
  });
}
