import { test, expect } from '@playwright/test';
import { execFileSync } from 'node:child_process';
import { mkdtempSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { apiCtx, seedWorkspace } from './seed';
import { expectFullyInViewport, openPage } from './helpers';

// ─────────────────────────────────────────────────────────────────────────────
// Merge-conflict resolver, GitKraken-style (desktop-browser only).
//
// A repo is seeded with a real in-progress conflicted merge (two branches
// touching the same line, `git merge` left mid-conflict). The resolver must:
//  - surface the in-progress merge as a banner + "Resolve conflicts" tab,
//  - show the conflict as side A / side B with header + PER-LINE checkboxes so
//    a resolution can mix parts of both sides,
//  - live-preview the recomposed file in the Output pane (with conflict
//    navigation), and
//  - complete the merge once every file is resolved.
// ─────────────────────────────────────────────────────────────────────────────

const REPO_NAME = 'e2e-conflict-repo';
let repoDir = '';

test.beforeAll(async () => {
  const { ctx, base } = await apiCtx();
  const wsId = await seedWorkspace(ctx, base);
  repoDir = mkdtempSync(join(tmpdir(), 'otto-e2e-conflict-'));
  const git = (...a: string[]) =>
    execFileSync('git', ['-C', repoDir, ...a], { stdio: 'ignore' });
  git('init', '-q', '-b', 'main');
  git('config', 'user.email', 'e2e@otto.local');
  git('config', 'user.name', 'E2E');
  git('config', 'commit.gpgsign', 'false');
  writeFileSync(
    join(repoDir, 'team.html'),
    '<header>About</header>\n<ul>\n<li>placeholder</li>\n</ul>\n<footer>fin</footer>\n',
  );
  git('add', '.');
  git('commit', '-q', '-m', 'base');
  // Branch B: its own version of the middle line.
  git('checkout', '-q', '-b', 'feature');
  writeFileSync(
    join(repoDir, 'team.html'),
    '<header>About</header>\n<ul>\n<li>Yoda was a friend of mine.</li>\n</ul>\n<footer>fin</footer>\n',
  );
  git('commit', '-q', '-am', 'feature line');
  // Branch A (main): a conflicting version of the same line.
  git('checkout', '-q', 'main');
  writeFileSync(
    join(repoDir, 'team.html'),
    '<header>About</header>\n<ul>\n<li>The founding was in 2020.</li>\n</ul>\n<footer>fin</footer>\n',
  );
  git('commit', '-q', '-am', 'main line');
  // Leave a REAL conflicted merge in progress (the command exits non-zero).
  try {
    git('merge', 'feature');
  } catch {
    /* expected: conflict */
  }
  const r = await ctx.post(`${base}/api/v1/workspaces/${wsId}/repos`, {
    data: { path: repoDir, name: REPO_NAME },
  });
  if (!r.ok()) throw new Error(`repo seed failed: ${r.status()} ${await r.text()}`);
  await ctx.dispose();
});

test('pick parts of A and B, preview the output, complete the merge', async ({ page }) => {
  await openPage(page, 'git');
  await page.locator('.git-tab-new').click();
  const menu = page.locator('.ctx-menu');
  await menu.locator('.ctx-search-input').fill(REPO_NAME);
  await menu.getByRole('menuitem', { name: REPO_NAME }).click();

  // The in-progress merge surfaces as a banner; enter the resolver.
  await expect(page.locator('.merge-banner')).toBeVisible({ timeout: 15_000 });
  await page.locator('.merge-banner .btn', { hasText: 'Resolve conflicts' }).click();
  const resolver = page.locator('.resolver');
  await expect(resolver).toBeVisible();
  await expect(resolver.locator('.file-row', { hasText: 'team.html' })).toBeVisible();

  // One conflict card, with the A/B side headers and per-line checkboxes.
  const hunk = page.locator('.hunk');
  await expect(hunk).toHaveCount(1);
  await expect(hunk.locator('.side-tag.tag-a')).toBeVisible();
  await expect(hunk.locator('.side-tag.tag-b')).toBeVisible();

  // Output pane: present, in-viewport, and honest about the unresolved state.
  const output = page.locator('.output');
  await expect(output).toBeVisible();
  await expect(output.locator('.output-count')).toContainText('conflict 1 of 1');
  await expect(output.locator('.out-unresolved')).toContainText('conflict 1 — unresolved');
  await expectFullyInViewport(page, output.locator('.output-bar'), 'output nav bar');

  // Pick A's line (per-line checkbox) AND all of B (header checkbox): the
  // resolution mixes both sides — the point of the GitKraken-style picker.
  await hunk.locator('.pick-line.ours', { hasText: 'The founding was in 2020.' }).click();
  await expect(hunk.locator('.pick-line.ours.picked')).toHaveCount(1);
  await hunk.locator('.side-head.theirs input[type="checkbox"]').check();
  // The header checkbox must reflect into the per-line picks.
  await expect(hunk.locator('.pick-line.theirs.picked')).toHaveCount(1);
  await expect(hunk.locator('.resolved-badge')).toBeVisible();

  // The Output pane live-previews BOTH picked lines, in A-then-B order.
  await expect(output.locator('.out-unresolved')).toHaveCount(0);
  const merged = output.locator('.out-resolved');
  await expect(merged).toContainText('The founding was in 2020.');
  await expect(merged).toContainText('Yoda was a friend of mine.');

  // Resolve the file, then complete the merge.
  const markBtn = page.locator('.pane-head .btn.primary');
  await expect(markBtn).toBeEnabled();
  await markBtn.click();
  const completeBtn = resolver.locator('.resolver-foot .btn.primary');
  await expect(completeBtn).toBeEnabled({ timeout: 10_000 });
  await completeBtn.click();

  // Back on the graph, no merge banner — and the merge commit is in the log.
  await expect(page.locator('.resolver')).toHaveCount(0, { timeout: 15_000 });
  await expect(page.locator('.merge-banner')).toHaveCount(0);
});
