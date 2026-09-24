import { test, expect } from '@playwright/test';
import { apiCtx, seedWorkspace } from './seed';

// ─────────────────────────────────────────────────────────────────────────────
// ⌃-letters inside a focused terminal belong to the shell (EOF, kill-line,
// readline motion). The global key map lets ⌃ stand in for ⌘ elsewhere (non-Mac
// remote clients), but in a terminal ⌃D must not split the pane, ⌃K must not
// open the command bar, and ⌃B must not toggle the sidebar.
// ─────────────────────────────────────────────────────────────────────────────

test.setTimeout(180_000);

test('⌃D / ⌃K / ⌃B in a focused terminal reach the shell, not the app', async ({ page }, info) => {
  test.skip(info.project.name !== 'desktop-browser', 'desktop-browser project only');
  const { ctx, base } = await apiCtx();
  const ws = await seedWorkspace(ctx, base);
  const r = await ctx.post(`${base}/api/v1/workspaces/${ws}/sessions`, {
    data: { kind: 'agent', provider: 'shell', title: 'Ctrl keys', cwd: '/tmp', meta: { origin: 'e2e' } },
  });
  if (!r.ok()) throw new Error(`seed session → ${r.status()} ${await r.text()}`);
  const id = (await r.json()).id as string;
  await page.addInitScript((w) => {
    localStorage.setItem('otto_workspace', w as string);
    localStorage.setItem('otto_firstrun_dismissed', '1');
  }, ws);

  await page.goto(`/#/agents/${id}`);
  const term = page.locator(`[data-session="${id}"] .xterm`).first();
  await expect(term).toBeVisible({ timeout: 150_000 });
  await term.click();
  const panes = page.locator('[data-pane-key]');
  const before = await panes.count();
  const sidebarBefore = await page.locator('.shell').getAttribute('class');

  await page.keyboard.type('echo ctrlkeys-ok');
  await page.keyboard.press('Control+a'); // readline: start of line
  await page.keyboard.press('Control+d'); // non-empty line: deletes a char (never EOF here)
  await page.keyboard.press('Control+b'); // readline: back one char
  await page.keyboard.press('Control+k'); // readline: kill to end of line

  await expect(panes).toHaveCount(before); // no split
  await expect(page.locator('.palette')).toHaveCount(0);
  await expect(page.getByTestId('floating-bar')).not.toHaveAttribute('data-presence', 'full');
  expect(await page.locator('.shell').getAttribute('class')).toBe(sidebarBefore);
  await ctx.dispose();
});
