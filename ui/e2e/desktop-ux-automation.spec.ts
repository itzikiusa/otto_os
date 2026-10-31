import { apiCtx, seedWorkspace } from './seed';
import { test, expect } from '@playwright/test';
import { expectNoHorizontalOverflow, expectFullyInViewport } from './helpers';

for (const route of ['workflows']) {
  test(`${route}: no workspace offers an actionable setup state`, async ({ page }) => {
    await page.route('**/api/v1/workspaces', r => r.fulfill({ json: [] }));
    await page.setViewportSize({ width: 375, height: 812 });
    await page.addInitScript(() => localStorage.setItem('otto_scheme', 'light'));
    await page.goto(`/#/${route}`);
    await expect(page.getByRole('heading', { name: 'Add a workspace to get started' })).toBeVisible();
    const add = page.getByTestId('page-empty').getByRole('button', { name: 'Add workspace', exact: true });
    await expectFullyInViewport(page, add);
    await expectNoHorizontalOverflow(page);
    await add.focus();
    await page.keyboard.press('Enter');
    await expect(page.getByRole('dialog')).toBeVisible();
    await page.keyboard.press('Escape');
    await expect(page.getByRole('dialog')).toHaveCount(0);
    await page.screenshot({ path: `/tmp/otto-ux-screenshots/automation-after-phone-${route}.png` });
  });
}

test('personal agents: keyboard tab selection moves focus to the selected tab', async ({ page }) => {
  await page.route('**/api/v1/workspaces', r => r.fulfill({ json: [] }));
  await page.goto('/#/personal-agents');
  const agents = page.getByRole('tab', { name: 'Agents', exact: true });
  const rooms = page.getByRole('tab', { name: 'Agent channels', exact: true });
  await agents.focus();
  await page.keyboard.press('ArrowRight');
  await expect(rooms).toHaveAttribute('aria-selected', 'true');
  await expect(rooms).toBeFocused();
  await page.keyboard.press('Home');
  const tasks = page.getByRole('tab', { name: 'Tasks', exact: true });
  await expect(tasks).toHaveAttribute('aria-selected', 'true');
  await expect(tasks).toBeFocused();
});


for (const [route, endpoint] of [
  ['loops', 'goal-loops'],
  ['workflows', 'workflows'], ['scheduled-tasks', 'scheduled-tasks'],
  ['personal-agents', 'personal-agents'],
]) {
  test(`${route}: a failed load recovers through inline Retry`, async ({ page }) => {
    const { ctx, base } = await apiCtx();
    const wsId = await seedWorkspace(ctx, base);
    await ctx.dispose();
    await page.addInitScript(id => localStorage.setItem('otto_workspace', id), wsId);
    let failing = true;
    const attempts: number[] = [];
    await page.route(`**/api/v1/workspaces/${wsId}/${endpoint}*`, async r => {
      attempts.push(Date.now());
      if (failing) await r.fulfill({ status: 503, json: { code: 'unavailable', message: 'Service temporarily unavailable' } });
      else await r.continue();
    });
    await page.goto(`/#/${route}`);
    const retry = page.getByRole('button', { name: 'Retry', exact: true }).first();
    await expect(retry).toBeVisible({ timeout: 20000 });
    const priorAttempts = attempts.length;
    failing = false;
    await retry.click();
    await expect.poll(() => attempts.length).toBeGreaterThan(priorAttempts);
    await expect(retry).toHaveCount(0);
    await expectNoHorizontalOverflow(page);
  });
}

for (const [theme, scheme, width, height, rtl] of [
  ['native', 'light', 1440, 900, false], ['native', 'dark', 1440, 900, false],
  ['warm', 'dark', 1440, 900, false], ['native', 'dark', 1024, 768, true],
] as const) {
  test(`setup states: ${theme} ${scheme} ${width} RTL=${rtl}`, async ({ page }) => {
    await page.route('**/api/v1/workspaces', r => r.fulfill({ json: [] }));
    await page.setViewportSize({ width, height });
    await page.addInitScript(({ theme, scheme }) => {
      localStorage.setItem('otto_theme', theme);
      localStorage.setItem('otto_scheme', scheme);
    }, { theme, scheme });
    for (const [route, title] of [
      ['workflows', 'Workflows'],
    ]) {
      await page.goto(`/#/${route}`);
      if (rtl) await page.evaluate(() => document.documentElement.dir = 'rtl');
      // Hash navigation retains the outgoing page while the next chunk loads.
      // All three pages share the empty-state copy, so first identify the destination.
      await expect(page.getByRole('heading', { name: title, exact: true, level: 1 })).toBeVisible();
      await expect(page.getByRole('heading', { name: 'Add a workspace to get started' })).toBeVisible();
      await expectFullyInViewport(page, page.getByTestId('page-empty').getByRole('button', { name: 'Add workspace', exact: true }));
      await expectNoHorizontalOverflow(page);
      await page.screenshot({ path: `/tmp/otto-ux-screenshots/automation-after-${theme}-${scheme}-${width}-${route}.png` });
    }
  });
}
