import { test, expect } from '@playwright/test';
import { expectFullyInViewport, expectNoHorizontalOverflow } from './helpers';

// The PR's agent thread on phones and tablets (MOCK mode — mock-runtime.ts):
// the left column does not exist ≤1024 px, so the thread opens in a
// full-height sheet from the PR header's ⋯, and nothing scrolls sideways.

test.beforeEach(async ({ page }, ti) => {
  test.skip(ti.project.name.startsWith('desktop'), 'phones and tablets');
  await page.addInitScript(() => localStorage.setItem('otto_mock', '1'));
});

test('the agent thread opens in a sheet from ⋯ and fits the screen', async ({ page }) => {
  await page.goto('/#/git/rep_otto/pr/51');
  await expect(page.locator('.prd-title')).toBeVisible({ timeout: 15_000 });
  await expect(page.getByTestId('pr-agent-panel')).toHaveCount(0); // no column here

  await page.getByRole('button', { name: 'More pull request actions' }).click();
  await page.locator('.ctx-menu').getByText('Ask a question', { exact: true }).click();
  const sheet = page.getByRole('dialog', { name: 'Agent · PR #51' });
  await expect(sheet).toBeVisible();
  const ask = sheet.getByRole('textbox', { name: 'Ask about this pull request' });
  await expectFullyInViewport(page, ask, 'ask box');
  await expectNoHorizontalOverflow(page);

  await sheet.getByRole('button', { name: 'Explain this PR' }).click();
  await expect(sheet.getByTestId('embedded-runtime-thread')).toBeVisible({ timeout: 10_000 });
  await expectNoHorizontalOverflow(page);

  // Closing the sheet leaves the PR where it was.
  await sheet.getByRole('button', { name: 'Close Agent · PR #51' }).click();
  await expect(sheet).toBeHidden();
  await expect(page.locator('.prd-title')).toBeVisible();
});
