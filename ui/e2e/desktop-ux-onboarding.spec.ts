import { test, expect } from '@playwright/test';
import { expectFullyInViewport } from './helpers';

for (const direction of ['ltr', 'rtl']) {
  test(`first-run setup keeps controls reachable on a phone (${direction})`, async ({ page }) => {
    await page.setViewportSize({ width: 375, height: 812 });
    await page.addInitScript((dir) => localStorage.setItem('otto_direction', dir), direction);
    await page.route('**/api/v1/workspaces', (route) => route.fulfill({ json: [] }));
    await page.goto('/#/agents');
    const coach = page.locator('.coach');
    await expect(coach).toBeVisible();
    expect((await coach.boundingBox())!.width).toBeLessThanOrEqual(375);
    await expect.poll(async () => coach.evaluate((el) => el.scrollWidth - el.clientWidth)).toBeLessThanOrEqual(2);
    const browse = coach.getByRole('button', { name: 'Browse…', exact: true });
    await browse.scrollIntoViewIfNeeded();
    await expectFullyInViewport(page, browse);
    await expect(coach.getByPlaceholder('my-project', { exact: true })).toHaveAccessibleName('Workspace name');
    await expect(coach.getByPlaceholder('~/code/my-project')).toHaveAccessibleName('Workspace folder');
  });
}
