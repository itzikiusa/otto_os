import { test, expect } from '@playwright/test';
import { execFileSync } from 'node:child_process';
import { mkdtempSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { apiCtx, seedWorkspace } from './seed';
import { expectFullyInViewport, openPage } from './helpers';

// ─────────────────────────────────────────────────────────────────────────────
// GitKraken-style WIP flow on the commit graph (desktop-browser only).
//
// The separate "Changes" tab is gone: a dirty working tree renders as a dashed
// "// WIP" row pinned above the graph, and selecting it opens the staging /
// commit panel in the right detail pane. This spec drives the full loop:
// dirty repo → WIP row → stage via the panel → commit → WIP row gone + the new
// commit at the top of the graph.
// ─────────────────────────────────────────────────────────────────────────────

const REPO_NAME = 'e2e-wip-repo';
let repoDir = '';

test.beforeAll(async () => {
  const { ctx, base } = await apiCtx();
  const wsId = await seedWorkspace(ctx, base);
  repoDir = mkdtempSync(join(tmpdir(), 'otto-e2e-wip-'));
  const git = (...a: string[]) =>
    execFileSync('git', ['-C', repoDir, ...a], { stdio: 'ignore' });
  git('init', '-q');
  git('config', 'user.email', 'e2e@otto.local');
  git('config', 'user.name', 'E2E');
  git('config', 'commit.gpgsign', 'false');
  writeFileSync(join(repoDir, 'app.txt'), 'hello\n');
  writeFileSync(join(repoDir, 'lib.txt'), 'lib v1\n');
  git('add', '.');
  git('commit', '-q', '-m', 'init');
  // Dirty working tree: one tracked modification + one untracked file.
  writeFileSync(join(repoDir, 'app.txt'), 'hello world\n');
  writeFileSync(join(repoDir, 'notes.txt'), 'todo\n');
  const r = await ctx.post(`${base}/api/v1/workspaces/${wsId}/repos`, {
    data: { path: repoDir, name: REPO_NAME },
  });
  if (!r.ok()) throw new Error(`repo seed failed: ${r.status()} ${await r.text()}`);
  await ctx.dispose();
});

async function openRepo(
  page: import('@playwright/test').Page,
  name = REPO_NAME,
): Promise<void> {
  await openPage(page, 'git');
  // Open the seeded repo via the "+" picker (idempotent: already-open repos
  // show as a tab instead — click that).
  const existingTab = page.locator('.git-tab-name', { hasText: name });
  if (await existingTab.count()) {
    await existingTab.first().click();
  } else {
    await page.locator('.git-tab-new').click();
    const menu = page.locator('.ctx-menu');
    await menu.locator('.ctx-search-input').fill(name);
    // .first(): the picker can list the same repo under more than one group
    // (e.g. a recents section) — any entry opens the same tab.
    await menu.getByRole('menuitem', { name }).first().click();
  }
  await expect(page.locator('.rv-tabs')).toBeVisible();
}

test('WIP row → stage → commit, all on the graph', async ({ page }) => {
  await openRepo(page);

  // There is no Changes/History tab any more; the Graph tab carries the dirty
  // count badge instead.
  const tabs = page.locator('.rv-tab');
  await expect(tabs.filter({ hasText: 'Graph' })).toBeVisible();
  await expect(tabs.filter({ hasText: 'Changes' })).toHaveCount(0);
  await expect(tabs.filter({ hasText: 'History' })).toHaveCount(0);

  // The dirty working tree renders as the dashed WIP row above the graph.
  const wipRow = page.locator('.wip-row');
  await expect(wipRow).toBeVisible({ timeout: 15_000 });
  await expect(wipRow).toContainText('// WIP');
  await expect(wipRow).toContainText('2 files changed');

  // Selecting it opens the staging panel in the right detail pane.
  await wipRow.click();
  const panel = page.locator('.wip-panel');
  await expect(panel).toBeVisible();
  await expectFullyInViewport(page, panel, 'WIP staging panel');

  // Both files sit under Unstaged; nothing staged yet. (exact: 'Staged Files'
  // is a substring of 'Unstaged Files' for Playwright's default matcher.)
  await expect(panel.getByText('Unstaged Files', { exact: true })).toBeVisible();
  await expect(panel.getByText('Staged Files', { exact: true })).toBeVisible();
  await expect(panel.locator('.wp-file')).toHaveCount(2);
  await expect(panel.getByText('Nothing staged yet.')).toBeVisible();

  // Clicking a file name opens its working diff inline in the panel.
  await panel.locator('.wp-name', { hasText: 'app.txt' }).click();
  await expect(panel.locator('.wp-diff')).toBeVisible();
  await expect(panel.locator('.wp-diff')).toContainText('app.txt');
  await panel.locator('.wp-diff-head .wp-close').click();

  // "Stage all" moves both files into Staged and arms the commit button.
  await panel.getByText('Stage all', { exact: true }).click();
  await expect(panel.getByText('Nothing unstaged.')).toBeVisible({ timeout: 10_000 });
  const commitBtn = panel.locator('.btn.primary');
  await expect(commitBtn).toContainText('Commit (2)');
  await expect(commitBtn).toBeDisabled(); // no summary yet

  await panel.locator('.subject-input').fill('feat: wip panel e2e commit');
  await expect(commitBtn).toBeEnabled();
  await commitBtn.click();

  // Commit lands: the WIP row disappears (tree clean), the panel auto-closes,
  // and the new commit tops the graph.
  await expect(page.locator('.wip-row')).toHaveCount(0, { timeout: 15_000 });
  await expect(page.locator('.wip-panel')).toHaveCount(0);
  await expect(
    page.locator('.graph-row', { hasText: 'feat: wip panel e2e commit' }),
  ).toBeVisible({ timeout: 15_000 });
});

test('selecting a file in a LARGE changeset shows its diff immediately', async ({ page }) => {
  // Regression (2026-07-16): the per-file diff was appended INSIDE the tree
  // scroller, so on a big changeset it rendered below hundreds of file rows —
  // far off-screen — and clicking a file looked like a no-op. The diff is now
  // a sibling flex region: seed enough files to overflow the pane and assert
  // the diff header actually lands in the viewport (toBeVisible alone passes
  // for content scrolled out of view, which is how this shipped unnoticed).
  //
  // Self-contained repo under a UNIQUE name: fullyParallel runs this file's
  // tests in separate workers, each with its own beforeAll seed — sharing
  // REPO_NAME/repoDir would race the commit test's worker.
  const bigName = 'e2e-wip-big-repo';
  const dir = mkdtempSync(join(tmpdir(), 'otto-e2e-wip-big-'));
  const git = (...a: string[]) =>
    execFileSync('git', ['-C', dir, ...a], { stdio: 'ignore' });
  git('init', '-q');
  git('config', 'user.email', 'e2e@otto.local');
  git('config', 'user.name', 'E2E');
  git('config', 'commit.gpgsign', 'false');
  writeFileSync(join(dir, 'base.txt'), 'base\n');
  git('add', '.');
  git('commit', '-q', '-m', 'init');
  for (let i = 0; i < 120; i++) {
    writeFileSync(join(dir, `bulk-${String(i).padStart(3, '0')}.txt`), `bulk ${i}\n`);
  }
  const { ctx, base } = await apiCtx();
  const wsId = await seedWorkspace(ctx, base);
  const r = await ctx.post(`${base}/api/v1/workspaces/${wsId}/repos`, {
    data: { path: dir, name: bigName },
  });
  if (!r.ok()) throw new Error(`repo seed failed: ${r.status()} ${await r.text()}`);
  await ctx.dispose();

  await openRepo(page, bigName);
  const wipRow = page.locator('.wip-row');
  await expect(wipRow).toBeVisible({ timeout: 15_000 });
  await wipRow.click();
  const panel = page.locator('.wip-panel');
  await expect(panel).toBeVisible();

  await panel.locator('.wp-name', { hasText: 'bulk-000.txt' }).click();
  const head = panel.locator('.wp-diff-head');
  await expectFullyInViewport(page, head, 'per-file diff header');
  // An untracked file renders as an all-added diff, not "No textual diff".
  await expect(panel.locator('.wp-diff')).toContainText('bulk 0');
});
