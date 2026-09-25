import { test, expect, type Page } from '@playwright/test';
import { apiCtx, seedWorkspace } from './seed';
import { expectFullyInViewport, expectNoHorizontalOverflow } from './helpers';

let workspaceId = '';
test.beforeAll(async () => {
  const { ctx, base } = await apiCtx();
  workspaceId = await seedWorkspace(ctx, base);
});
async function boot(page: Page, mobile = false): Promise<void> {
  if (mobile) await page.setViewportSize({ width: 390, height: 844 });
  await page.addInitScript((id) => {
    localStorage.setItem('otto_workspace', id);
    localStorage.setItem('otto_home_rotate', '0');
    localStorage.setItem('otto_home_views', JSON.stringify([
      { id: 'one', name: 'First', boxes: [] }, { id: 'two', name: 'Second', boxes: [] },
    ]));
    localStorage.removeItem('otto_bar_spaces');
  }, workspaceId);
  await page.goto('/#/home');
  await expect(page.getByRole('tab', { name: 'First', exact: true })).toBeVisible();
}
test('phone navigator owns focus and restores its trigger', async ({ page }) => {
  await boot(page, true);
  const trigger = page.getByRole('button', { name: 'Open navigator', exact: true });
  await trigger.focus();
  await trigger.press('Enter');
  const drawer = page.getByRole('dialog', { name: 'Navigator', exact: true });
  await expect(drawer).toBeVisible();
  await expect.poll(() => drawer.evaluate((el) => el.contains(document.activeElement))).toBe(true);
  const close = drawer.getByRole('button', { name: 'Close Navigator' });
  await close.focus();
  await close.press('Shift+Tab');
  await expect.poll(() => drawer.evaluate((el) => el.contains(document.activeElement))).toBe(true);
  await page.keyboard.press('Escape');
  await expect(drawer).toBeHidden();
  await expect(trigger).toBeFocused();
  await expectNoHorizontalOverflow(page);
});
test('shared prompt has an accessible field name', async ({ page }) => {
  await boot(page);
  await page.getByRole('button', { name: 'Add space', exact: true }).click();
  await expect(page.locator('.cf-input')).toHaveAccessibleName('Space name');
});
test('Home space shortcuts do not act through a dialog', async ({ page }) => {
  await boot(page);
  await page.getByRole('button', { name: 'Add widget', exact: true }).click();
  const dialog = page.getByRole('dialog', { name: 'Add widget' });
  await expect(dialog).toBeVisible();
  await dialog.getByRole('button', { name: 'Close', exact: true }).focus();
  await page.keyboard.press('ArrowRight');
  await expect(page.getByRole('tab', { name: 'First', exact: true })).toHaveAttribute('aria-selected', 'true');
});
test('Plain English palette traps focus and Escape works from its buttons', async ({ page }) => {
  await boot(page);
  await page.keyboard.press('Control+i');
  const palette = page.getByRole('dialog', { name: 'Command palette' });
  await expect(palette).toBeVisible();
  await palette.locator('textarea').fill('Review my work');
  await palette.getByRole('button', { name: /Plan it/ }).focus();
  await page.keyboard.press('Tab');
  await expect.poll(() => palette.evaluate((el) => el.contains(document.activeElement))).toBe(true);
  await palette.getByRole('button', { name: 'Optimize prompt' }).focus();
  await page.keyboard.press('Escape');
  await expect(palette).toBeHidden();
});
test('Needs you expands every source in place', async ({ page }) => {
  await page.route('**/mcp/approvals?status=pending', (route) => route.fulfill({ json: Array.from({ length: 5 }, (_, i) => ({
    id: `approval-${i}`, title: `Approval ${i + 1}`, created_at: new Date().toISOString(), workspace_id: workspaceId,
  })) }));
  await boot(page);
  const needs = page.locator('[data-card="needs"]');
  await expect(needs.getByRole('button', { name: '2 more', exact: true })).toBeVisible();
  await needs.getByRole('button', { name: '2 more', exact: true }).click();
  await expect(needs.getByRole('button', { name: /^Approval 5/ })).toBeVisible();
  await expect(page).toHaveURL(/#\/home$/);
});

for (const variant of [
  { scheme: 'light', theme: 'native', direction: 'ltr', mobile: false },
  { scheme: 'dark', theme: 'native', direction: 'ltr', mobile: false },
  { scheme: 'dark', theme: 'warm', direction: 'rtl', mobile: true },
]) {
  test(`Home and prompt fit ${variant.theme} ${variant.scheme} ${variant.direction} ${variant.mobile ? 'phone' : 'desktop'}`, async ({ page }, info) => {
    await page.addInitScript((v) => {
      localStorage.setItem('otto_scheme', v.scheme);
      localStorage.setItem('otto_theme', v.theme);
      localStorage.setItem('otto_direction', v.direction);
    }, variant);
    if (!variant.mobile) await page.setViewportSize({ width: 1440, height: 900 });
    await boot(page, variant.mobile);
    await page.getByRole('button', { name: 'Add space', exact: true }).click();
    const dialog = page.getByRole('dialog', { name: 'New space 03' });
    await expectFullyInViewport(page, dialog);
    await expectNoHorizontalOverflow(page);
    await page.getByRole('textbox', { name: 'Space name', exact: true }).fill('A long space name '.repeat(8));
    await expectFullyInViewport(page, dialog);
    await page.screenshot({ path: info.outputPath('home-prompt.png'), fullPage: true });
  });
}
