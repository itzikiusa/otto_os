import { test, expect, type Page } from '@playwright/test';
import { execFileSync } from 'node:child_process';
import { mkdtempSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { apiCtx, seedWorkspace } from './seed';
import { openPage } from './helpers';

// ─────────────────────────────────────────────────────────────────────────────
// R0 — a branch switch NEVER pulls or merges (desktop-browser only).
//
// The bug this locks down: checking out `develop` from the graph ran
// stash → checkout → `git pull --no-rebase` → pop, so a `develop` that was
// both ahead of and behind origin came back with an unasked-for MERGE COMMIT
// out of a gesture the user read as "go look at that branch".
//
// Fixture: a bare origin + a clone whose `develop` is ahead AND behind, with a
// dirty file that overlaps the switch (so git refuses and the stash offer
// appears). The assertions read GIT ITSELF — one parent on the tip, still
// behind origin — because "it merged" is exactly what a toast would hide.
//
// One test, run in sequence: the three gestures share one on-disk repo, and
// splitting them across parallel workers would race the same working tree.
// ─────────────────────────────────────────────────────────────────────────────

const REPO_NAME = 'e2e-switch-repo';
let repoDir = '';
let repoId = '';
let apiBase = '';

function git(dir: string, ...args: string[]): string {
  return execFileSync('git', ['-C', dir, ...args], { encoding: 'utf8' }).trim();
}

const SHARED_BASE = 'l1\nl2\nl3\nl4\nl5\n';

test.beforeAll(async () => {
  const { ctx, base } = await apiCtx();
  apiBase = base;
  const wsId = await seedWorkspace(ctx, base);

  const root = mkdtempSync(join(tmpdir(), 'otto-e2e-switch-'));
  const origin = join(root, 'origin.git');
  const seed = join(root, 'seed');
  repoDir = join(root, 'work');

  execFileSync('git', ['init', '-q', '--bare', '-b', 'main', origin]);
  execFileSync('git', ['init', '-q', '-b', 'main', seed]);
  git(seed, 'config', 'user.email', 'e2e@otto.local');
  git(seed, 'config', 'user.name', 'E2E');
  git(seed, 'config', 'commit.gpgsign', 'false');
  writeFileSync(join(seed, 'shared.txt'), SHARED_BASE);
  git(seed, 'add', '-A');
  git(seed, 'commit', '-q', '-m', 'init');
  git(seed, 'branch', 'develop');
  git(seed, 'branch', 'feature');
  git(seed, 'remote', 'add', 'origin', origin);
  git(seed, 'push', '-q', '-u', 'origin', 'main', 'develop', 'feature');

  execFileSync('git', ['clone', '-q', origin, repoDir]);
  git(repoDir, 'config', 'user.email', 'e2e@otto.local');
  git(repoDir, 'config', 'user.name', 'E2E');
  git(repoDir, 'config', 'commit.gpgsign', 'false');

  // Local `develop` is AHEAD of origin/develop (its own commit, which also
  // makes shared.txt differ from main so the dirty switch is refused)…
  git(repoDir, 'checkout', '-q', 'develop');
  writeFileSync(join(repoDir, 'shared.txt'), SHARED_BASE.replace('l5', 'DEVELOP'));
  git(repoDir, 'commit', '-q', '-am', 'develop edits the last line');
  git(repoDir, 'checkout', '-q', 'main');

  // …and BEHIND it too, so a pull here would produce a merge commit.
  git(seed, 'checkout', '-q', 'develop');
  writeFileSync(join(seed, 'upstream_only.txt'), 'theirs\n');
  git(seed, 'add', '-A');
  git(seed, 'commit', '-q', '-m', 'upstream develop work');
  git(seed, 'push', '-q', 'origin', 'develop');
  git(repoDir, 'fetch', '-q', 'origin');

  // The uncommitted change: a DIFFERENT line of the same file, so the stash
  // pops back cleanly while git still refuses the plain checkout.
  writeFileSync(join(repoDir, 'shared.txt'), SHARED_BASE.replace('l1', 'DIRTY'));

  const r = await ctx.post(`${base}/api/v1/workspaces/${wsId}/repos`, {
    data: { path: repoDir, name: REPO_NAME },
  });
  if (!r.ok()) throw new Error(`repo seed failed: ${r.status()} ${await r.text()}`);
  repoId = ((await r.json()) as { id: string }).id;
  await ctx.dispose();
});

async function openRepo(page: Page): Promise<void> {
  await openPage(page, 'git');
  const existingTab = page.locator('.git-tab-name', { hasText: REPO_NAME });
  if (await existingTab.count()) {
    await existingTab.first().click();
  } else {
    await page.locator('.git-tab-new').click();
    const menu = page.locator('.ctx-menu');
    await menu.locator('.ctx-search-input').fill(REPO_NAME);
    await menu.getByRole('menuitem', { name: REPO_NAME }).first().click();
  }
  await expect(page.locator('.rv-tabs')).toBeVisible();
  await expect(page.locator('.refs-panel')).toBeVisible({ timeout: 20_000 });
}

/** `GET /repos/{id}/status` from the daemon — `behind` comes from
 *  `--porcelain=v2 --branch`, so a bare on-disk origin needs no network. */
async function status(): Promise<{ branch: string; ahead: number; behind: number }> {
  const { ctx } = await apiCtx();
  const r = await ctx.get(`${apiBase}/api/v1/repos/${repoId}/status`);
  const body = (await r.json()) as { branch: string; ahead: number; behind: number };
  await ctx.dispose();
  return body;
}

function localRow(page: Page, name: string) {
  return page.locator('.ref-row:not(.remote):not(.tag):not(.stash-row)', { hasText: name }).first();
}

test('a switch stashes and restores, never pulls — and drag is still the only merge', async ({
  page,
}) => {
  await openRepo(page);
  expect((await status()).branch).toBe('main');

  // ── 1. Dirty switch to `develop`: git refuses, the offer says in so many
  //       words that nothing is pulled, and the switch leaves no merge behind.
  await localRow(page, 'develop').dblclick();

  const stashDialog = page.locator('.sheet[role="dialog"][aria-label="Stash, switch & restore"]');
  await expect(stashDialog).toBeVisible({ timeout: 20_000 });
  await expect(stashDialog).toContainText('Nothing is pulled or merged');
  await stashDialog.getByRole('button', { name: 'Stash & switch' }).click();

  await expect(page.locator('.toast', { hasText: 'Switched to develop' })).toBeVisible({
    timeout: 20_000,
  });

  expect(git(repoDir, 'rev-parse', '--abbrev-ref', 'HEAD')).toBe('develop');
  expect(git(repoDir, 'log', '-1', '--format=%P').split(/\s+/).filter(Boolean)).toHaveLength(1);
  expect(git(repoDir, 'log', '-1', '--format=%s')).toBe('develop edits the last line');
  const after = await status();
  expect(after.branch).toBe('develop');
  expect(after.behind).toBeGreaterThan(0); // nothing was fetched or merged
  expect(git(repoDir, 'status', '--porcelain')).toContain('shared.txt'); // restored

  // The restored change is back in the working tree → the WIP row returns.
  await expect(page.locator('.wip-row')).toBeVisible({ timeout: 20_000 });

  // ── 2. The commit context menu offers a PLAIN checkout and never the old
  //       pulling gesture.
  // `.graph-row` also matches the WIP row (which has no context menu) — target
  // the develop tip by its subject.
  await page
    .locator('.graph-row', { hasText: 'develop edits the last line' })
    .first()
    .click({ button: 'right' });
  const menu = page.locator('.ctx-menu');
  await expect(menu).toBeVisible();
  await expect(menu).not.toContainText('stash · pull · pop');
  await expect(menu.getByRole('menuitem', { name: 'Pull develop' })).toBeVisible();
  await page.keyboard.press('Escape');
  await expect(menu).toHaveCount(0);

  // ── 3. A remote-only row checks out as a LOCAL TRACKING branch from the same
  //       dirty tree (this used to dead-end: the offer was `create:false` only).
  await page.locator('.ref-row.remote', { hasText: 'feature' }).first().dblclick();
  if (await stashDialog.isVisible().catch(() => false)) {
    await stashDialog.getByRole('button', { name: 'Stash & switch' }).click();
  }
  await expect(page.locator('.toast', { hasText: 'feature' })).toBeVisible({ timeout: 20_000 });
  expect(git(repoDir, 'rev-parse', '--abbrev-ref', 'HEAD')).toBe('feature');
  expect(git(repoDir, 'rev-parse', '--abbrev-ref', 'feature@{u}')).toBe('origin/feature');

  // ── 4. Dragging a branch onto another is still the one and only merge
  //       gesture, and it still goes through the approval modal.
  await localRow(page, 'feature').dragTo(localRow(page, 'develop'));
  await expect(page.locator('.sheet[role="dialog"][aria-label="Merge branch"]')).toBeVisible({
    timeout: 20_000,
  });
});
