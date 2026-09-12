import { test, expect } from '@playwright/test';

// ─────────────────────────────────────────────────────────────────────────────
// PR merge modal (R4) — desktop-browser only.
//
// The isolated e2e daemon has no forge account, so no real PR is reachable:
// this spec drives the UI in MOCK mode (`localStorage.otto_mock`, see
// ui/src/lib/api/mock.ts), whose fixture PR #42 ships a failing `lint` check
// and a review with 2 blocker findings. That is exactly the shape the modal
// exists for — it must refuse the merge, name the reasons, and only release the
// button once "Merge anyway" is ticked.
// ─────────────────────────────────────────────────────────────────────────────

test.beforeEach(async ({ page }, testInfo) => {
  test.skip(testInfo.project.name !== 'desktop-browser', 'desktop-browser only');
  await page.addInitScript(() => {
    localStorage.setItem('otto_mock', '1');
    localStorage.setItem('otto_rail_expanded', '0');
    // The PR tab is remembered per PR — start every run on Summary, where the
    // action row (and its Merge button) lives.
    localStorage.removeItem('otto_pr_tab_rep_otto_42');
  });
});

test('merge modal blocks on failing CI + blockers, then merges with "Merge anyway"', async ({
  page,
}) => {
  await page.goto('/#/git/rep_otto/pr/42');
  await expect(page.locator('.shell')).toBeVisible({ timeout: 15_000 });

  // Merge no longer posts straight from the action row — it opens the modal.
  await page.locator('.prd-actions').getByRole('button', { name: 'Merge' }).click();
  const modal = page.getByRole('dialog', { name: 'Merge pull request' });
  await expect(modal).toBeVisible();

  // Readiness rows: the per-check CI list, and the blocker count from the review.
  await expect(modal.locator('.check').filter({ hasText: 'build' })).toBeVisible();
  await expect(modal.locator('.check').filter({ hasText: 'lint' })).toBeVisible();
  await expect(modal.getByText('2 of 3 unresolved')).toBeVisible();

  // Blocked, with both reasons spelled out under the button.
  const mergeBtn = modal.getByRole('button', { name: 'Merge', exact: true });
  await expect(mergeBtn).toBeDisabled();
  const reasons = modal.locator('.reasons');
  await expect(reasons).toContainText('CI failing');
  await expect(reasons).toContainText('2 blocker findings');

  // "Merge anyway" is the only way past it.
  await modal.getByText('Merge anyway').click();
  await expect(mergeBtn).toBeEnabled();

  // The delete-source-branch choice rides along with the merge.
  await modal.getByText('Delete source branch after merge').click();
  await mergeBtn.click();

  await expect(page.locator('.toast-title').filter({ hasText: 'PR merged' })).toBeVisible();
  // The modal closes and the PR reloads as merged.
  await expect(modal).toBeHidden();
});
