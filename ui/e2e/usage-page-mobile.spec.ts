// Usage on a phone: the first screen reads top to bottom (what is left, the
// month, the chart, Where it went, then the details card); the By-model rows
// are two lines that never scroll sideways, and the switch is finger-sized.
import { test, expect, type Page } from '@playwright/test';
import { routeUsage } from './usage-page-fixture';
import { expectNoHorizontalOverflow } from './helpers';

test.use({ serviceWorkers: 'block' });
const phone = (page: Page) => page.evaluate(() => window.innerWidth <= 640);

test('model rows stay inside their card and the switch takes a finger', async ({ page }) => {
  await routeUsage(page);
  await page.goto('/#/usage');
  await expect(page.locator('.shell')).toBeVisible({ timeout: 30_000 });
  test.skip(!(await phone(page)), 'phone layouts only');
  await expect(page.getByTestId('usage-spend')).toContainText('on track for');
  const tab = page.getByTestId('usage-where-model');
  expect((await tab.boundingBox())!.height).toBeGreaterThanOrEqual(44);
  await tab.tap();
  const models = page.getByTestId('usage-models');
  await expect(models.locator('.cost-row')).toHaveCount(4);
  const vw = page.viewportSize()!.width;
  for (const row of await models.locator('.cost-row').all()) {
    await row.scrollIntoViewIfNeeded();
    const cost = (await row.locator('.bar-cost').boundingBox())!;
    expect(cost.x + cost.width, 'cost inside the screen').toBeLessThanOrEqual(vw);
    expect(await row.evaluate((el) => el.scrollWidth <= el.clientWidth + 1), 'row does not scroll sideways').toBe(true);
  }
  await expect(page.getByTestId('usage-details')).toContainText('Budgets');
  await expectNoHorizontalOverflow(page);
});
