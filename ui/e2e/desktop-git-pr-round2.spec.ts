import { test, expect } from '@playwright/test';
import { expectFullyInViewport, expectNoHorizontalOverflow } from './helpers';

// ─────────────────────────────────────────────────────────────────────────────
// PR workspace, round 2 (desktop-browser, MOCK mode — ui/src/lib/api/mock-prs.ts,
// mock-pr-comments.ts): the files follow a push, "Changes since your review",
// "Review requested from you" in the list, and editing reviewers (asks first,
// naming who is added and removed).
// ─────────────────────────────────────────────────────────────────────────────

test.beforeEach(async ({ page }, testInfo) => {
  test.skip(testInfo.project.name !== 'desktop-browser', 'desktop-browser only');
  await page.addInitScript(() => {
    localStorage.setItem('otto_mock', '1');
    localStorage.setItem('otto_rail_expanded', '0');
  });
  await page.setViewportSize({ width: 1600, height: 950 });
});

test('a push while the PR is open: the files follow the new head and the reader is told', async ({ page }) => {
  await page.clock.install();
  await page.goto('/#/git/rep_otto/pr/51');
  const prd = page.locator('.prd');
  await expect(prd).toHaveAttribute('data-diff-head', /^c1fac680/, { timeout: 15_000 });
  await expect(page.getByTestId('pr-head-moved')).toHaveCount(0);

  // Someone pushes; the open PR's background check (every ~2 min) sees it.
  await page.evaluate(() => localStorage.setItem('otto_mock_pr_pushed', '51'));
  await page.clock.fastForward(150_000);

  // No Refresh click: the diff (and the Files panel fed by it) is re-read at
  // the new head, and the pill says what happened.
  await expect(prd).toHaveAttribute('data-diff-head', /^c7e57a11/, { timeout: 15_000 });
  const pill = page.getByTestId('pr-head-moved');
  await expect(pill).toHaveText(/1 new commit · files updated/);
  await pill.click();
  await expect(pill).toHaveCount(0);
});

test('Changes since your review: offered after a push, shows only the range', async ({ page }) => {
  // You reviewed #51 at its old head; the author pushed since.
  await page.addInitScript(() => {
    localStorage.setItem('otto_pr_reviewed_head:rep_otto#51', 'c1fac680');
    localStorage.setItem('otto_mock_pr_pushed', '51');
  });
  await page.goto('/#/git/rep_otto/pr/51');
  await expect(page.locator('.prd')).toHaveAttribute('data-diff-head', /^c7e57a11/, { timeout: 15_000 });
  await page.getByRole('tab', { name: 'Code', exact: true }).click();
  const since = page.getByTestId('pr-since-review');
  await expect(since).toBeVisible();
  await since.click();
  await expect(page.locator('.prd-commit-label')).toHaveText('Changes since your review');
  await expect(since).toHaveCount(0);

  // The same view from the commit picker, with "All commits" to go back.
  await page.locator('.prd-commit-pick').click();
  const menu = page.getByRole('menu');
  await expect(menu.getByRole('menuitemcheckbox', { name: /Changes since your review/ })).toHaveAttribute('aria-checked', 'true');
  await menu.getByRole('menuitemcheckbox', { name: /All commits/ }).click();
  await expect(page.locator('.prd-commit-label')).toHaveText('All commits');
  await expectNoHorizontalOverflow(page);
});

test('approving records the head you reviewed at; nothing is offered until a push', async ({ page }) => {
  await page.goto('/#/git/rep_otto/pr/51');
  await expect(page.locator('.prd')).toHaveAttribute('data-diff-head', /^c1fac680/, { timeout: 15_000 });
  const head = page.locator('.prd-actions');
  await head.getByRole('button', { name: 'Approve', exact: true }).click();
  await page.getByRole('dialog').getByRole('button', { name: 'Approve PR' }).click();
  await expect(head.getByText('Approved', { exact: true })).toBeVisible();
  await expect
    .poll(() => page.evaluate(() => localStorage.getItem('otto_pr_reviewed_head:rep_otto#51')))
    .toMatch(/^c1fac680/);
  await page.getByRole('tab', { name: 'Code', exact: true }).click();
  await expect(page.getByTestId('pr-since-review')).toHaveCount(0);
});

test('Review requested from you lists only PRs that ask you, with its own empty state', async ({ page }) => {
  await page.goto('/#/git/rep_otto/pr/51');
  await expect(page.locator('.prd-title')).toBeVisible({ timeout: 15_000 });
  const rows = page.locator('.pr-row');
  await expect(rows.filter({ hasText: 'VIP program' })).toHaveCount(1);

  await page.getByRole('button', { name: /^Filters/ }).click();
  const item = page.getByRole('menuitemcheckbox', { name: /Review requested from you/ });
  await expectFullyInViewport(page, item, 'the review filter item');
  await item.click();
  // Your own PRs never ask you for a review.
  await expect(rows.filter({ hasText: 'VIP program' })).toHaveCount(0);
  await expect(page.getByRole('button', { name: 'Filters (1 on)' })).toBeVisible();

  // The filter survives a reload (per-repo preference) and clears from the menu.
  await page.reload();
  await expect(page.getByRole('button', { name: 'Filters (1 on)' })).toBeVisible({ timeout: 15_000 });
  await page.getByRole('button', { name: 'Filters (1 on)' }).click();
  await page.getByRole('menuitem', { name: /Clear filters/ }).click();
  await expect(rows.filter({ hasText: 'VIP program' })).toHaveCount(1);
});

test('Edit reviewers asks first, naming who is added and who is removed', async ({ page }) => {
  await page.goto('/#/git/rep_otto/pr/51');
  await expect(page.locator('.prd-title')).toBeVisible({ timeout: 15_000 });
  const side = page.getByRole('complementary', { name: 'Pull request details' });
  await expect(side.getByText('maya')).toBeVisible();
  await side.getByTestId('pr-reviewers-edit').click();

  const sheet = page.getByRole('dialog', { name: 'Reviewers of #51' });
  await expect(sheet.getByTestId('pr-reviewers-save')).toBeDisabled();
  await sheet.getByRole('textbox', { name: 'Add a reviewer' }).fill('carl');
  await sheet.getByRole('button', { name: 'Add', exact: true }).click();
  await sheet.getByRole('button', { name: 'Remove maya' }).click();
  await sheet.getByTestId('pr-reviewers-save').click();

  const confirm = page.getByRole('dialog').filter({ hasText: 'Update the reviewers of PR #51?' });
  await expect(confirm).toContainText('Add: carl');
  await expect(confirm).toContainText('Remove: maya');
  await expect(confirm).toContainText('carl is asked for a review');
  await confirm.getByRole('button', { name: 'Update reviewers' }).click();

  await expect(sheet).toHaveCount(0);
  await expect(side.getByText('carl')).toBeVisible();
  await expect(side.getByText('maya')).toHaveCount(0);
});

test('Edit reviewers: cancelling the confirm sends nothing', async ({ page }) => {
  await page.goto('/#/git/rep_otto/pr/51');
  await expect(page.locator('.prd-title')).toBeVisible({ timeout: 15_000 });
  const side = page.getByRole('complementary', { name: 'Pull request details' });
  await side.getByTestId('pr-reviewers-edit').click();
  const sheet = page.getByRole('dialog', { name: 'Reviewers of #51' });
  await sheet.getByRole('button', { name: 'Remove maya' }).click();
  await sheet.getByTestId('pr-reviewers-save').click();
  const confirm = page.getByRole('dialog').filter({ hasText: 'Update the reviewers of PR #51?' });
  await confirm.getByRole('button', { name: 'Cancel' }).click();
  // Back in the sheet with the edit intact; the PR itself is unchanged.
  await expect(sheet).toBeVisible();
  await expect(sheet.getByRole('button', { name: 'Keep maya' })).toBeVisible();
  await sheet.getByRole('button', { name: 'Cancel' }).click();
  await page.reload();
  await expect(page.getByRole('complementary', { name: 'Pull request details' }).getByText('maya')).toBeVisible({ timeout: 15_000 });
});
