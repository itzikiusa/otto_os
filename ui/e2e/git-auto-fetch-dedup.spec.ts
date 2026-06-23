import { test, expect, type Page } from '@playwright/test';
import { execFileSync } from 'node:child_process';
import { mkdtempSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { apiCtx, seedWorkspace } from './seed';

// E2E for the two Git features:
//   1. Auto-fetch — a quiet background `git fetch` for the OPEN repo tabs every
//      few seconds. Verifies the loop actually fires a fetch, that the visible
//      ahead/behind chip updates when the upstream advances, that the toggle
//      persists, and that pausing it stops the fetches.
//   2. De-dup — registering the same repository path twice returns the SAME repo
//      (no duplicate row) so the Git page never opens a second identical tab.
//
// The auto-fetch interval is forced to 2s via the persisted `otto_git_auto_fetch`
// localStorage key so the loop fires quickly under test.

let workspaceId = '';
// A repo whose origin/<branch> is AHEAD of the local clone, so a `git fetch`
// turns the local repo "behind" — letting us assert the chip updates live.
let upstreamRepoId = '';

const git = (dir: string, ...args: string[]) =>
  execFileSync('git', ['-C', dir, ...args], { stdio: 'ignore' });

/** Make a fresh local git repo with one commit. Returns its dir. */
function initRepo(prefix: string): string {
  const dir = mkdtempSync(join(tmpdir(), prefix));
  git(dir, 'init', '-q');
  git(dir, 'config', 'user.email', 'e2e@otto.local');
  git(dir, 'config', 'user.name', 'E2E');
  git(dir, 'config', 'commit.gpgsign', 'false');
  writeFileSync(join(dir, 'README.md'), 'hello\n');
  git(dir, 'add', '-A');
  git(dir, 'commit', '-q', '-m', 'initial');
  return dir;
}

/**
 * Seed a working repo bound to a bare "remote" that is one commit AHEAD of the
 * working clone's remote-tracking ref. Before a fetch the repo reports behind=0;
 * after the auto-fetch runs, origin/<branch> advances and the repo reports
 * behind=1 — exactly the signal we assert in the UI.
 */
async function seedUpstreamAheadRepo(): Promise<string> {
  const { ctx, base } = await apiCtx();
  // 1) A working repo with an initial commit.
  const work = initRepo('otto-e2e-af-work-');
  const branch = execFileSync('git', ['-C', work, 'rev-parse', '--abbrev-ref', 'HEAD'], {
    encoding: 'utf8',
  }).trim();
  // 2) A bare remote; push the working repo to it.
  const bare = mkdtempSync(join(tmpdir(), 'otto-e2e-af-bare-'));
  git(bare, 'init', '--bare', '-q');
  git(work, 'remote', 'add', 'origin', bare);
  git(work, 'push', '-q', '-u', 'origin', branch);
  // 3) Advance the bare via a throwaway second clone so origin/<branch> is now
  //    ahead of the working repo's (still stale) remote-tracking ref.
  const pusher = mkdtempSync(join(tmpdir(), 'otto-e2e-af-push-'));
  execFileSync('git', ['clone', '-q', bare, pusher], { stdio: 'ignore' });
  git(pusher, 'config', 'user.email', 'e2e@otto.local');
  git(pusher, 'config', 'user.name', 'E2E');
  git(pusher, 'config', 'commit.gpgsign', 'false');
  writeFileSync(join(pusher, 'README.md'), 'hello\nupstream advance\n');
  git(pusher, 'add', '-A');
  git(pusher, 'commit', '-q', '-m', 'upstream advance');
  git(pusher, 'push', '-q', 'origin', `HEAD:${branch}`);
  // 4) Register the WORKING repo.
  const r = await ctx.post(`${base}/api/v1/workspaces/${workspaceId}/repos`, {
    data: { path: work, name: 'e2e-upstream' },
  });
  if (!r.ok()) throw new Error(`register upstream repo → ${r.status()} ${await r.text()}`);
  const repo = (await r.json()) as { id: string };
  await ctx.dispose();
  return repo.id;
}

test.beforeAll(async () => {
  const { ctx, base } = await apiCtx();
  workspaceId = await seedWorkspace(ctx, base);
  await ctx.dispose();
  upstreamRepoId = await seedUpstreamAheadRepo();
});

/** Seed localStorage BEFORE the app boots: workspace + a fast (2s) auto-fetch
 *  config (enabled unless `paused`). */
async function boot(page: Page, opts: { paused?: boolean } = {}): Promise<void> {
  await page.addInitScript(
    ([wsId, cfg]) => {
      localStorage.setItem('otto_workspace', wsId as string);
      localStorage.setItem('otto_rail_expanded', '0');
      localStorage.setItem('otto_git_auto_fetch', cfg as string);
    },
    [workspaceId, JSON.stringify({ enabled: !opts.paused, intervalSec: 2 })] as const,
  );
}

const isFetch = (url: string) => /\/repos\/[^/]+\/fetch$/.test(url);

// ────────────────────────────────────────────────────────────────────────────
// AUTO-FETCH
// ────────────────────────────────────────────────────────────────────────────

test('auto-fetch: fires a background git fetch for the open repo', async ({ page }) => {
  await boot(page);
  await page.goto(`/#/git/${upstreamRepoId}/graph`);
  await expect(page.locator('.gitpage')).toBeVisible({ timeout: 30_000 });
  // The loop (2s interval) issues a POST …/repos/{id}/fetch on its own.
  const req = await page.waitForRequest(
    (r) => r.method() === 'POST' && isFetch(r.url()),
    { timeout: 20_000 },
  );
  expect(isFetch(req.url())).toBe(true);
});

test('auto-fetch: updates the behind chip after the upstream advances', async ({ page }) => {
  await boot(page);
  await page.goto(`/#/git/${upstreamRepoId}/graph`);
  await expect(page.locator('.gitpage')).toBeVisible({ timeout: 30_000 });
  // The seeded upstream is 1 commit ahead → once the auto-fetch runs, the
  // toolbar branch chip shows a "behind" marker (↓). Poll generously: a couple
  // of 2s rounds + git fetch latency.
  await expect(page.locator('.branch-chip .ab.down')).toBeVisible({ timeout: 25_000 });
  await expect(page.locator('.branch-chip .ab.down')).toHaveText(/↓\s*1/);
});

test('auto-fetch: toggle is on by default and pausing it persists', async ({ page }) => {
  await boot(page);
  await page.goto(`/#/git/${upstreamRepoId}/graph`);
  await expect(page.locator('.gitpage')).toBeVisible({ timeout: 30_000 });
  const toggle = page.locator('.git-autofetch');
  await expect(toggle).toBeVisible({ timeout: 15_000 });
  await expect(toggle).toHaveClass(/\bon\b/);
  await expect(toggle).toHaveAttribute('aria-pressed', 'true');

  // Pause it → state flips and is persisted to localStorage.
  await toggle.click();
  await expect(toggle).not.toHaveClass(/\bon\b/);
  await expect(toggle).toHaveAttribute('aria-pressed', 'false');
  const persisted = await page.evaluate(() =>
    JSON.parse(localStorage.getItem('otto_git_auto_fetch') ?? '{}'),
  );
  expect(persisted.enabled).toBe(false);
});

test('auto-fetch: paused → no background fetch is issued', async ({ page }) => {
  await boot(page, { paused: true });
  let fetches = 0;
  page.on('request', (r) => {
    if (r.method() === 'POST' && isFetch(r.url())) fetches++;
  });
  await page.goto(`/#/git/${upstreamRepoId}/graph`);
  await expect(page.locator('.gitpage')).toBeVisible({ timeout: 30_000 });
  await expect(page.locator('.git-autofetch')).not.toHaveClass(/\bon\b/);
  // Wait well past two 2s rounds; the paused loop must never POST a fetch.
  await page.waitForTimeout(6_000);
  expect(fetches, 'paused auto-fetch must not issue any fetch').toBe(0);
});

// ────────────────────────────────────────────────────────────────────────────
// DE-DUP
// ────────────────────────────────────────────────────────────────────────────

test('dedup: registering the same path twice returns the same repo (no duplicate)', async () => {
  const { ctx, base } = await apiCtx();
  const dir = initRepo('otto-e2e-dedup-');

  const before = (await (await ctx.get(`${base}/api/v1/git/repos`)).json()) as unknown[];

  const reg = async () => {
    const r = await ctx.post(`${base}/api/v1/workspaces/${workspaceId}/repos`, {
      data: { path: dir, name: 'e2e-dedup' },
    });
    expect(r.ok(), `register → ${r.status()}`).toBe(true);
    return (await r.json()) as { id: string };
  };
  const first = await reg();
  const second = await reg();
  // Even a SUBDIRECTORY of the repo must dedup to the same root.
  const subdir = join(dir, 'nested');
  execFileSync('mkdir', ['-p', subdir]);
  const r3 = await ctx.post(`${base}/api/v1/workspaces/${workspaceId}/repos`, {
    data: { path: subdir, name: 'e2e-dedup-sub' },
  });
  const third = (await r3.json()) as { id: string };

  expect(second.id).toBe(first.id);
  expect(third.id).toBe(first.id);

  const after = (await (await ctx.get(`${base}/api/v1/git/repos`)).json()) as unknown[];
  expect(after.length, 'exactly one new repo row should be created').toBe(before.length + 1);
  await ctx.dispose();
});

test('dedup: opening an already-open repo keeps a single tab', async ({ page }) => {
  await boot(page);
  await page.goto(`/#/git/${upstreamRepoId}/graph`);
  await expect(page.locator('.gitpage')).toBeVisible({ timeout: 30_000 });
  await expect(page.locator('.git-tab')).toHaveCount(1, { timeout: 15_000 });
  // Re-opening the same repo (same hash route) must not mint a second tab.
  await page.goto(`/#/git/${upstreamRepoId}/history`);
  await expect(page.locator('.gitpage')).toBeVisible({ timeout: 30_000 });
  await expect(page.locator('.git-tab')).toHaveCount(1);
});
