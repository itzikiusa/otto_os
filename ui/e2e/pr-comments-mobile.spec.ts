import { test, expect } from '@playwright/test';
import { expectFullyInViewport, expectNoHorizontalOverflow } from './helpers';

// PR comments on phones and tablets (MOCK mode — ui/src/lib/api/mock-prs.ts):
// reactions and your comment's menu are finger-sized, the reaction picker
// stays on screen, the merge-checks line sits in the header (the side column
// is far below on a phone), and a Bitbucket PR says it has no reactions
// instead of offering controls that would fail.

test.beforeEach(async ({ page }, ti) => {
  test.skip(ti.project.name.startsWith('desktop'), 'phones and tablets');
  await page.addInitScript(() => localStorage.setItem('otto_mock', '1'));
});

// Wider than 1024 px (iPad landscape) is the desktop layout.
const phoneWidth = (page: import('@playwright/test').Page): boolean => (page.viewportSize()?.width ?? 0) <= 1024;

test('reactions and the comment menu are touch-sized; the picker stays on screen', async ({ page }) => {
  test.skip(!phoneWidth(page), 'desktop layout at this width');
  await page.goto('/#/git/rep_otto/pr/51');
  await expect(page.locator('.prd-title')).toBeVisible({ timeout: 15_000 });
  const thread = page.locator('.prd-comments .prd-thread').filter({ hasText: 'thresholds' });
  const pill = thread.getByRole('button', { name: 'Thumbs up: 2 reactions' });
  await pill.scrollIntoViewIfNeeded();
  const add = thread.getByRole('button', { name: 'Add a reaction' }).first();
  for (const b of [pill, add, page.getByRole('button', { name: 'Comment actions' }).first()]) {
    const box = (await b.boundingBox())!;
    expect(box.height, 'touch target height').toBeGreaterThanOrEqual(44);
  }
  await add.click();
  const menu = page.locator('.ctx-menu');
  await expect(menu).toBeVisible();
  await expectFullyInViewport(page, menu, 'reaction picker');
  await menu.getByText('Heart').click();
  await expect(thread.getByRole('button', { name: 'Heart: 1 reaction, including yours' })).toBeVisible();
  await expectNoHorizontalOverflow(page);
});

test('the header shows the merge-checks line', async ({ page }) => {
  test.skip(!phoneWidth(page), 'the side column is beside the summary at this width');
  await page.goto('/#/git/rep_otto/pr/51');
  await expect(page.locator('.prd-title')).toBeVisible({ timeout: 15_000 });
  const line = page.getByTestId('pr-merge-checks-line');
  await expect(line).toBeVisible();
  await expect(line).toContainText('1 of 3 checks passed · 1 task open');
  await expectNoHorizontalOverflow(page);
});

test('Bitbucket Cloud: no reaction controls, one quiet note', async ({ page }) => {
  await page.goto('/#/git/rep_wallet/pr/7');
  await expect(page.locator('.prd-title')).toBeVisible({ timeout: 15_000 });
  await expect(page.getByTestId('pr-no-reactions')).toHaveText('Bitbucket Cloud has no reactions');
  await expect(page.getByRole('button', { name: 'Add a reaction' })).toHaveCount(0);
});
