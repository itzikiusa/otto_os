import { test, expect } from '@playwright/test';
import { expectFullyInViewport, expectNoHorizontalOverflow } from './helpers';

// Phones and tablets (MOCK mode — mock-runtime.ts): the PR's agent thread
// lives in a sheet, so its state shows on the control that opens it. With
// the sheet closed a hidden status-only panel keeps the thread's embed
// reporting, and the opener's dot + label say "working" in words.

test.beforeEach(async ({ page }, ti) => {
  test.skip(ti.project.name.startsWith('desktop'), 'phones and tablets');
  await page.addInitScript(() => localStorage.setItem('otto_mock', '1'));
});

test('the agent opener opens the sheet and shows the thread working while it is closed', async ({ page }) => {
  await page.goto('/#/git/rep_otto/pr/51');
  await expect(page.locator('.prd-title')).toBeVisible({ timeout: 15_000 });

  const opener = page.getByTestId('pr-agent-open');
  await expect(opener).toHaveAttribute('aria-label', 'Agent thread');
  await expectFullyInViewport(page, opener, 'the agent opener');
  await opener.click();
  const sheet = page.getByRole('dialog', { name: 'Agent · PR #51' });
  await expect(sheet).toBeVisible();
  await sheet.getByRole('button', { name: 'Explain this PR' }).click();
  await expect(sheet.getByTestId('embedded-runtime-thread')).toBeVisible({ timeout: 10_000 });

  await sheet.getByRole('button', { name: 'Close Agent · PR #51' }).click();
  await expect(sheet).toBeHidden();
  // The thread keeps reporting from the hidden status-only panel.
  await expect(page.getByTestId('pr-agent-status')).toHaveCount(1);
  await page.evaluate(() => {
    const rt = (window as unknown as { __mockRuntime: { creates: unknown[]; status: (sid: string, s: 'running' | 'ready') => void } }).__mockRuntime;
    rt.status(`rts_mock${rt.creates.length}`, 'running');
  });
  await expect(opener).toHaveAttribute('data-status', 'working', { timeout: 10_000 });
  await expect(opener).toHaveAttribute('aria-label', 'Agent thread: the agent is working');
  await expect(opener.locator('.prw-agent-dot')).toBeVisible();
  await expectNoHorizontalOverflow(page);

  // Opening the sheet again hands the thread back to the sheet's panel.
  await opener.click();
  await expect(sheet).toBeVisible();
  await expect(page.getByTestId('pr-agent-status')).toHaveCount(0);
});
