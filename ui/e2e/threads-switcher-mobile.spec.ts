import { test, expect, type APIRequestContext } from '@playwright/test';
import { mkdtempSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { apiCtx } from './seed';
import { expectFullyInViewport, expectNoHorizontalOverflow } from './helpers';

// The command sheet's thread group by touch (lib/paletteThreads.svelte.ts —
// a phone / tablet gets the palette sheet where a desktop gets the ⌘K bar):
// opened from the bottom bar's More › Commands on a phone (⌘K with an iPad
// keyboard), the waiting thread leads with its status word, a tap opens it,
// and nothing scrolls sideways.

const PROJECTS = ['iphone-portrait', 'ipad-portrait'];

let ctx: APIRequestContext;
let base = '';
let wsId = '';
let dir = '';

test.beforeAll(async () => {
  const c = await apiCtx();
  ctx = c.ctx;
  base = c.base;
  dir = mkdtempSync(join(tmpdir(), 'switcher-m-'));
  writeFileSync(join(dir, 'README.md'), '# switcher\n');
  const r = await ctx.post(`${base}/api/v1/workspaces`, { data: { name: 'Switcher phone', root_path: dir } });
  wsId = (await r.json()).id as string;
});

test.afterAll(async () => {
  await ctx?.dispose();
  if (dir) rmSync(dir, { recursive: true, force: true });
});

test('phone: the command sheet leads with the waiting thread; a tap opens it', async ({ page }, info) => {
  test.skip(!PROJECTS.includes(info.project.name), 'phone and tablet projects');
  await page.addInitScript((id) => localStorage.setItem('otto_workspace', id as string), wsId);
  const quiet = (await (await ctx.post(`${base}/api/v1/workspaces/${wsId}/runtime-sessions`, { data: { provider: 'codex', cwd: dir, title: `Quiet ${info.project.name}` } })).json()).id as string;
  const waits = (await (await ctx.post(`${base}/api/v1/workspaces/${wsId}/runtime-sessions`, { data: { provider: 'codex', cwd: dir, title: `Waits ${info.project.name}` } })).json()).id as string;
  expect((await ctx.post(`${base}/api/v1/runtime-sessions/${waits}/prompt`, { data: { text: 'ask' } })).ok()).toBeTruthy();
  await page.goto(`/#/threads/${quiet}`);
  await expect(page.getByTestId(`runtime-thread-${quiet}`)).toBeVisible({ timeout: 30_000 });
  const list = page.getByRole('dialog', { name: 'Command palette' });
  const row = list.locator(`[data-cmd="thread:${waits}"]`);
  await expect(async () => {
    if (info.project.name === 'iphone-portrait') {
      await page.getByRole('button', { name: 'More' }).last().tap();
      await page.getByRole('button', { name: 'Commands' }).tap();
    } else await page.keyboard.press('Meta+k');
    await expect(row).toBeVisible({ timeout: 2_000 });
  }).toPass({ timeout: 20_000 });
  await expect(list.locator('.pal-section-header').first()).toHaveText('Threads');
  await expect(row.locator('.pal-group')).toHaveText(/^(Approval|Needs you)$/);
  expect((await row.boundingBox())!.height, 'touch target').toBeGreaterThanOrEqual(43.5);
  await expectFullyInViewport(page, list, 'bar list (phone)');
  await expectNoHorizontalOverflow(page);
  const out = process.env.THRC_SHOTS;
  for (const scheme of ['light', 'dark'] as const) {
    await page.emulateMedia({ colorScheme: scheme });
    await page.screenshot({ path: out ? `${out}/switcher-${info.project.name}-${scheme}.png` : info.outputPath(`switcher-${scheme}.png`), animations: 'disabled' });
  }
  await page.emulateMedia({ colorScheme: 'light' });
  await row.tap();
  await expect(page.getByTestId(`runtime-thread-${waits}`)).toBeVisible({ timeout: 30_000 });
});
