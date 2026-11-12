import { test, expect } from '@playwright/test';
import { expectFullyInViewport, openPage } from './helpers';
import { seedDiagnose } from './connections-diagnose-mocks';

// Phone / tablet: the diagnosis thread opens as a full-height sheet over the
// Connections page (no side split to squeeze into), and closes back to it.
test('the diagnosis thread is a full-height sheet on small screens', async ({ page }) => {
  const f = await seedDiagnose(page);
  await openPage(page, 'connections');
  await page.locator('.conn-row', { hasText: f.name }).first().click();

  const panel = page.getByTestId('ssh-exit-panel');
  await expect(panel.getByTestId('ssh-exit-message')).toContainText('Could not resolve', { timeout: 20_000 });
  await panel.getByTestId('conn-diagnose').click();
  await expect(page.getByTestId('diagnose-consent')).toBeVisible();
  await page.getByTestId('diagnose-start').click();

  const aside = page.getByTestId('diagnose-aside');
  await expect(aside.locator('[data-testid="runtime-thread-rt-diag-1"]')).toBeVisible({ timeout: 15_000 });
  const vw = page.viewportSize();
  const box = await aside.boundingBox();
  if (vw && box && vw.width <= 640) {
    expect(Math.round(box.height)).toBeGreaterThanOrEqual(vw.height - 2);
    expect(Math.round(box.width)).toBeGreaterThanOrEqual(vw.width - 2);
  }
  await expectFullyInViewport(page, aside.getByRole('button', { name: 'Close — the thread is kept' }), 'the sheet’s Close');
  await aside.getByRole('button', { name: 'Close — the thread is kept' }).click();
  await expect(aside).toHaveCount(0);
  await f.dispose();
});
