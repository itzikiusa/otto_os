import { test, expect } from '@playwright/test';
import { expectNoHorizontalOverflow } from './helpers';

// ─────────────────────────────────────────────────────────────────────────────
// PR workspace (desktop-browser, MOCK mode — ui/src/lib/api/mock-prs.ts):
// list | open-PR tabs, the T3-style header, Summary / Timeline / Code, the
// Code tab's hide-tests / viewed / tree / commit picker, and opening several
// PRs together from the list. Same UI for GitHub, Bitbucket and GitLab —
// everything goes through the provider-neutral /repos/{id}/prs/* routes.
// ─────────────────────────────────────────────────────────────────────────────

test.beforeEach(async ({ page }, testInfo) => {
  test.skip(testInfo.project.name !== 'desktop-browser', 'desktop-browser only');
  await page.addInitScript(() => {
    localStorage.setItem('otto_mock', '1');
    localStorage.setItem('otto_rail_expanded', '0');
  });
  await page.setViewportSize({ width: 1600, height: 950 });
});

test('a PR deep link opens in the workspace with header, stats and timeline', async ({ page }) => {
  await page.goto('/#/git/rep_otto/pr/51');
  await expect(page.locator('.prw')).toBeVisible({ timeout: 15_000 });
  await expect(page.locator('.prd-title')).toContainText('VIP program');
  // The route is handed back to the tab store; the PR stays open as a tab.
  await expect.poll(() => page.evaluate(() => location.hash)).toBe('#/git');
  await expect(page.locator('.prw-tab')).toHaveCount(1);
  await expect(page.locator('.prd-stats')).toContainText('23 files');
  await expect(page.locator('.prd-actions').getByRole('button', { name: 'Merge', exact: true })).toBeVisible();
  // Grouped list: my PRs first.
  await expect(page.locator('.prlist')).toContainText('Authored by you');

  await page.getByRole('tab', { name: 'Timeline', exact: true }).click();
  await expect(page.locator('.tlw')).toContainText('opened this pull request');
  await expect(page.locator('.tlw')).toContainText('pushed 1 commit');
  await expect(page.locator('.prd-code')).toBeHidden();
  await expectNoHorizontalOverflow(page);
});

test('Code tab: hide tests, viewed marks persist, tree, one commit', async ({ page }) => {
  await page.goto('/#/git/rep_otto/pr/51');
  await expect(page.locator('.prd-title')).toBeVisible({ timeout: 15_000 });
  await page.getByRole('tab', { name: 'Code', exact: true }).click();
  const heads = page.locator('.dfile-head');
  await expect(heads).toHaveCount(23);
  // >5 files: every file starts collapsed, like a reviewable file list.
  await expect(heads.first()).toHaveAttribute('aria-expanded', 'false');

  // Tests across languages (.spec.ts, _test.go, tests/test_*.py, src/test/…Test.java).
  await page.getByRole('button', { name: 'Hide test files' }).click();
  await expect(heads).toHaveCount(17);
  await expect(page.locator('.diff-root')).not.toContainText('levels_test.go');

  // Viewed: ticking folds the file and survives a reload.
  const viewed = page.locator('.prd-code').getByRole('checkbox', { name: /viewed/i }).first();
  await viewed.check();
  await expect(page.locator('.diff-root')).toContainText('1 / 17 viewed');
  await page.reload();
  await page.getByRole('tab', { name: 'Code', exact: true }).click();
  await expect(page.locator('.diff-root')).toContainText('1 / 17 viewed');

  // The diff toolbar stays pinned while the PR pane scrolls a long diff.
  await page.getByRole('button', { name: /expand all/i }).first().click();
  const tb = page.locator('.diff-toolbar.pr-toolbar');
  const paneTop = (await page.locator('.prd-body').boundingBox())!.y;
  await page.locator('.prd-body').evaluate((el) => { el.scrollTop += 2000; });
  await expect.poll(async () => Math.round((await tb.boundingBox())!.y - paneTop)).toBe(0);
  await expect(tb.getByRole('textbox', { name: /search/i })).toBeInViewport();

  await page.getByRole('button', { name: 'Show file tree' }).click();
  await expect(page.locator('.diff-nav .nav-file').first()).toBeVisible();

  // One commit: the picker swaps the diff to that commit's files.
  await page.locator('.prd-commit-pick').first().click();
  await page.locator('.ctx-menu').getByText('active pins page').click();
  await expect(page.locator('.prd-commit-pick').first()).toContainText('active pins page');
  await expect(heads.first()).toBeVisible();
  expect(await heads.count()).toBeLessThan(17);
});

test('Files panel under the list: a tree that jumps into Code and shares Viewed marks', async ({ page }) => {
  await page.goto('/#/git/rep_otto/pr/51');
  await expect(page.locator('.prd-title')).toBeVisible({ timeout: 15_000 });
  const panel = page.getByRole('region', { name: 'Files in pull request #51' });
  await expect(panel).toContainText('0/23 viewed');
  // A real tree: folders fold their files away.
  const dir = panel.locator('.nav-dir-row').first();
  await expect(dir).toHaveAttribute('aria-expanded', 'true');
  const before = await panel.locator('.nav-file').count();
  await dir.click();
  await expect(dir).toHaveAttribute('aria-expanded', 'false');
  expect(await panel.locator('.nav-file').count()).toBeLessThan(before);
  await dir.click();

  // A file click brings the Code tab forward and jumps to that file.
  const first = panel.locator('.nav-file').first();
  const path = (await first.getAttribute('title')) ?? '';
  await first.locator('.nav-file-open').click();
  await expect(page.getByRole('tab', { name: 'Code', exact: true })).toHaveAttribute('aria-selected', 'true');
  await expect(page.locator(`.prd-code .dfile-head[title="${path}"], .prd-code .dfile-head:has-text("${path.split('/').pop()}")`).first()).toBeInViewport();

  // Ticking in the panel is the diff's Viewed mark (and the other way round).
  await first.getByRole('checkbox').check();
  await expect(panel).toContainText('1/23 viewed');
  await expect(page.locator('.diff-root')).toContainText('1 / 23 viewed');
  // Hide viewed drops it from the tree once the reader moves on.
  await panel.getByRole('button', { name: 'Next unviewed file', exact: true }).click();
  await panel.getByRole('button', { name: 'Hide viewed files' }).click();
  await expect(panel.locator(`.nav-file[title="${path}"]`)).toHaveCount(0);
});

test('Approve / Request changes live in the header and update at once', async ({ page }) => {
  await page.goto('/#/git/rep_otto/pr/51');
  await expect(page.locator('.prd-title')).toBeVisible({ timeout: 15_000 });
  const head = page.locator('.prd-actions');
  const side = page.getByRole('complementary', { name: 'Pull request details' });

  // Request changes: the header button opens the message box on Summary.
  await head.getByRole('button', { name: 'Request changes' }).click();
  await page.getByRole('textbox', { name: 'What needs to change' }).fill('Cap the retry loop.');
  await page.locator('.prd-request-changes').getByRole('button', { name: 'Request changes' }).click();
  await page.getByRole('dialog').getByRole('button', { name: 'Request changes' }).click();
  // No refresh needed: the header and the reviewer list change right away.
  await expect(head.getByText('Changes requested')).toBeVisible();
  await expect(side.getByText('Changes requested')).toBeVisible();

  // Approve flips your opinion.
  await head.getByRole('button', { name: 'Approve', exact: true }).click();
  await page.getByRole('dialog').getByRole('button', { name: 'Approve PR' }).click();
  await expect(head.getByText('Approved', { exact: true })).toBeVisible();
  await expect(head.getByText('Changes requested')).toHaveCount(0);
  await expect(side.getByText('Changes requested')).toHaveCount(0);
});

test('PR header never overlaps: repo · #n · state stay whole as the window narrows', async ({ page }) => {
  for (const width of [1600, 1440, 1280, 1100]) {
    await page.setViewportSize({ width, height: 900 });
    await page.goto('/#/git/rep_otto/pr/51');
    await expect(page.locator('.prd-title')).toBeVisible({ timeout: 15_000 });
    const boxes = await page.evaluate(() => {
      const r = (sel: string) => document.querySelector(sel)!.getBoundingClientRect();
      return { num: r('.prd-num'), state: r('.prd-state'), actions: r('.prd-actions'), slug: r('.prd-slug') };
    });
    const overlap = (a: DOMRect, b: DOMRect) => a.left < b.right - 1 && b.left < a.right - 1 && a.top < b.bottom - 1 && b.top < a.bottom - 1;
    expect(overlap(boxes.num, boxes.state), `#n over the state chip at ${width}px`).toBe(false);
    expect(overlap(boxes.state, boxes.actions), `state chip under the actions at ${width}px`).toBe(false);
    expect(boxes.slug.width, `repo name visible at ${width}px`).toBeGreaterThan(40);
    for (const name of ['Request changes', 'Approve', 'Merge']) {
      await expect(page.locator('.prd-actions').getByRole('button', { name, exact: true })).toBeInViewport({ ratio: 1 });
    }
  }
});

test('select several PRs in the list and open them together as tabs', async ({ page }) => {
  await page.goto('/#/git/rep_otto/pr/51');
  await expect(page.locator('.prd-title')).toBeVisible({ timeout: 15_000 });
  await expect(page.locator('.prw-tab')).toHaveCount(1);
  await page.getByRole('checkbox', { name: 'Select #42' }).check();
  await page.getByRole('checkbox', { name: 'Select #50' }).check();
  await page.getByRole('button', { name: /Open 2 together/ }).click();
  await expect(page.locator('.prw-tab')).toHaveCount(3);
  await expect(page.locator('.prw-tab.active')).toContainText('#42');
  // Close the front tab: the neighbour comes forward.
  await page.getByRole('button', { name: 'Close #42' }).click();
  await expect(page.locator('.prw-tab')).toHaveCount(2);
  // Search qualifiers narrow the list.
  await page.locator('.pr-search').fill('label:bug');
  await expect(page.locator('.pr-row')).toHaveCount(1);
});

test('⋯ menu offers merge strategies, agent sessions and copy actions', async ({ page }) => {
  await page.goto('/#/git/rep_otto/pr/51');
  await expect(page.locator('.prd-title')).toBeVisible({ timeout: 15_000 });
  await page.getByRole('button', { name: 'More pull request actions' }).click();
  const menu = page.locator('.ctx-menu');
  for (const label of ['Ask a question', 'Explain this PR', 'Squash and merge', 'Rebase and merge', 'Copy link', 'Open on GitHub']) {
    await expect(menu.getByText(label, { exact: true })).toBeVisible();
  }
  await menu.getByText('Squash and merge', { exact: true }).click();
  const modal = page.getByRole('dialog', { name: 'Merge pull request' });
  await expect(modal).toBeVisible();
  await expect(modal.locator('select')).toHaveValue('squash');
});
