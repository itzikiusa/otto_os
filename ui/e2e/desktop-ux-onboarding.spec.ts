import { test, expect } from '@playwright/test';
import { expectFullyInViewport } from './helpers';

for (const direction of ['ltr', 'rtl']) {
  test(`first-run setup keeps controls reachable on a phone (${direction})`, async ({ page }, info) => {
    await page.setViewportSize({ width: 375, height: 812 });
    await page.addInitScript((dir) => localStorage.setItem('otto_direction', dir), direction);
    await page.route('**/api/v1/workspaces', (route) => route.fulfill({ json: [] }));
    await page.goto('/#/agents');
    const coach = page.locator('.coach');
    await expect(coach).toBeVisible();
    // Session bootstrap may briefly replace the coach with its loading state.
    await expect.poll(async () => (await coach.boundingBox())?.width ?? Infinity).toBeLessThanOrEqual(375);
    await expect.poll(async () => coach.evaluate((el) => el.scrollWidth - el.clientWidth)).toBeLessThanOrEqual(2);
    await expect(coach.locator('.tool-chips .chip').first()).toBeVisible();
    await expect.poll(() => coach.locator('.tool-chips .chip').evaluateAll((chips) =>
      Math.max(0, ...chips.map((chip) => chip.scrollHeight - chip.clientHeight)),
    )).toBeLessThanOrEqual(2);
    const browse = coach.getByRole('button', { name: 'Browse…', exact: true });
    await browse.scrollIntoViewIfNeeded();
    await expectFullyInViewport(page, browse);
    await expect(coach.getByPlaceholder('my-project', { exact: true })).toHaveAccessibleName('Workspace name');
    await expect(coach.getByPlaceholder('~/code/my-project')).toHaveAccessibleName('Workspace folder');
    await page.locator('.coach-wrap').evaluate((el) => { el.scrollTop = 0; });
    await page.screenshot({ path: info.outputPath('onboarding.png') });
  });
}
