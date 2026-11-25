// "Available in" on a phone: a server shared from another workspace shows
// its badge in the list without a horizontal scroll, and the "Available in…"
// dialog (its workspace checklist capped and scrolling) fits the screen.
import { test, expect } from '@playwright/test';
import { join } from 'node:path';
import { apiCtx, seedWorkspace } from './seed';
import { expectFullyInViewport, expectNoHorizontalOverflow } from './helpers';

const SHOTS = process.env.OTTO_E2E_SHOTS ?? '';

test('a shared-in server and its Available in… dialog fit a phone', async ({ page }, info) => {
  const { ctx, base } = await apiCtx();
  const home = await seedWorkspace(ctx, base, `Home ${info.project.name}`);
  const name = `phone-shared-${info.project.name}`;
  const made = await ctx.post(`${base}/api/v1/workspaces/${home}/mcp/servers`, {
    data: { name, transport: 'stdio', command: 'true', all_workspaces: true },
  });
  expect(made.ok(), await made.text()).toBeTruthy();
  // Enough workspaces to overflow the checklist.
  for (let i = 0; i < 8; i++) await seedWorkspace(ctx, base, `Extra ${i} ${info.project.name}`);
  const here = await seedWorkspace(ctx, base, `Here ${info.project.name}`);
  await ctx.dispose();
  await page.addInitScript((id) => localStorage.setItem('otto_workspace', id as string), here);
  await page.goto('/#/mcp/servers');
  const row = page.getByTestId('mcp-server-row').filter({ hasText: name });
  await expect(row.getByTestId('mcp-server-scope')).toHaveText(`Shared from Home ${info.project.name}`, { timeout: 30_000 });
  await expectNoHorizontalOverflow(page);
  await row.getByRole('button', { name: `More actions for ${name}` }).click();
  await page.getByRole('menuitem', { name: 'Available in…' }).click();
  const dialog = page.getByRole('dialog').filter({ hasText: 'is available' });
  await expect(dialog.getByRole('option', { name: `Only Home ${info.project.name}` })).toBeAttached();
  await dialog.getByLabel('Available in').selectOption('selected');
  await expectFullyInViewport(page, dialog, 'Available in dialog');
  const picks = dialog.getByRole('group', { name: 'Workspaces to share with' });
  await expect.poll(() => picks.evaluate((el) => el.scrollHeight > el.clientHeight)).toBe(true);
  if (SHOTS) {
    for (const scheme of ['light', 'dark'] as const) {
      await page.emulateMedia({ colorScheme: scheme });
      await page.screenshot({ path: join(SHOTS, `phone-scope-dialog-${info.project.name}-${scheme}.png`) });
    }
  }
  await dialog.getByRole('button', { name: 'Cancel', exact: true }).click();
  await expect(dialog).toHaveCount(0);
});
