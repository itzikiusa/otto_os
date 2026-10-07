import { test, expect } from '@playwright/test';
import { apiCtx, seedWorkspace } from './seed';
import { expectFullyInViewport, expectNoHorizontalOverflow } from './helpers';

test('open workspace modal keeps its draft and focus when the desktop floating bar unmounts', async ({ page }) => {
  const { ctx, base } = await apiCtx();
  const workspace = await seedWorkspace(ctx, base);
  await ctx.dispose();
  await page.setViewportSize({ width: 1500, height: 900 });
  await page.addInitScript((id) => {
    localStorage.setItem('otto_workspace', id);
    localStorage.setItem('otto_rail_expanded', '1');
    localStorage.setItem('otto_firstrun_dismissed', '1');
  }, workspace);
  await page.goto('/#/agents');
  const trigger = page.locator('.navigator [data-nav-id="connections"]').first();
  await expect(trigger).toBeVisible();
  await expect(page.getByTestId('floating-bar')).toHaveCount(1);
  await trigger.focus();
  // Open the same production form used by the native menu, without submitting
  // a workspace or introducing a mock Modal/FloatingBar implementation.
  await page.evaluate(async () => {
    const path = '/src/lib/stores/ui.svelte.ts';
    const { ui } = await import(/* @vite-ignore */ path);
    ui.newWorkspaceOpen = true;
  });
  const sheet = page.getByRole('dialog', { name: 'Add Workspace', exact: true });
  const name = sheet.getByRole('textbox', { name: 'Name', exact: true });
  await expect(name).toBeFocused();
  await name.fill('Unsubmitted resize draft');
  for (const width of [1000, 750, 1500]) {
    await page.setViewportSize({ width, height: 900 });
    await expect(page.getByTestId('floating-bar')).toHaveCount(width > 1024 ? 1 : 0);
    // Let effect cleanup microtasks and the next render settle without moving focus.
    await page.evaluate(() => new Promise<void>((resolve) => requestAnimationFrame(() => requestAnimationFrame(() => resolve()))));
    await expect(name).toBeFocused();
    await expect(name).toHaveValue('Unsubmitted resize draft');
    await expectFullyInViewport(page, sheet);
    await expectNoHorizontalOverflow(page);
  }
  await sheet.getByRole('button', { name: 'Cancel', exact: true }).click();
  await expect(sheet).toBeHidden();
  await expect(trigger).toBeFocused();
});
