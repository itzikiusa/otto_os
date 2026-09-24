import { test, expect, type APIRequestContext } from '@playwright/test';
import { apiCtx, seedWorkspace } from './seed';

// ─────────────────────────────────────────────────────────────────────────────
// Keys typed INSIDE a session tab belong to the control that has focus
// (desktop-browser only).
//
// Regression: the tab's own keydown handler (Enter/Space activate, ←/→ Home/End
// switch tabs) also caught keys bubbling up from its children — so the rename
// field could not take a space (Space was preventDefault'ed) and ←/→ switched
// tabs mid-edit, and Enter on the focused × activated the tab instead of
// closing it.
// ─────────────────────────────────────────────────────────────────────────────

let ctx: APIRequestContext;
let base = '';
const ids: Record<string, string> = {};

test.beforeEach(async ({ page }, info) => {
  test.skip(info.project.name !== 'desktop-browser', 'desktop-browser project only');
  const c = await apiCtx();
  ctx = c.ctx;
  base = c.base;
  const wsId = await seedWorkspace(ctx, base);
  for (const title of ['Pirlo', 'Seedorf']) {
    const r = await ctx.post(`${base}/api/v1/workspaces/${wsId}/sessions`, {
      data: { kind: 'agent', provider: 'shell', title, cwd: '/tmp', meta: { origin: 'e2e' } },
    });
    if (!r.ok()) throw new Error(`seed ${title} → ${r.status()} ${await r.text()}`);
    ids[title] = (await r.json()).id as string;
  }
  await page.addInitScript((id) => {
    localStorage.setItem('otto_workspace', id as string);
    localStorage.removeItem('otto_close_tab_pref');
  }, wsId);
  await page.goto('/#/agents');
  await expect(page.getByText('Pirlo').first()).toBeVisible({ timeout: 20_000 });
});

test.afterEach(async () => {
  await ctx?.dispose();
});

async function openTab(page: import('@playwright/test').Page, title: string) {
  await page.locator('.nav-item.nested-item', { hasText: title }).first().click();
  const tab = page.locator('.tab', { hasText: title });
  await expect(tab).toBeVisible({ timeout: 10_000 });
  return tab;
}

test('the rename field takes spaces and arrow keys without switching tabs', async ({ page }) => {
  await openTab(page, 'Seedorf');
  await openTab(page, 'Pirlo');
  // By id: in rename mode the title text is an input value, not text content.
  const tab = page.locator(`.tab[data-tab-id="${ids.Pirlo}"]`);

  await tab.locator('.tab-title').dblclick();
  const field = tab.locator('.tab-rename');
  await field.click(); // the freshly focused terminal may take focus back first
  await expect(field).toBeFocused();
  await field.press('ControlOrMeta+a');
  await field.pressSequentially('Andrea Pirlo');
  // ←/→ move the caret; they must not activate the neighbouring tab.
  await field.press('ArrowLeft');
  await field.press('ArrowRight');
  await expect(field).toBeFocused();
  await expect(field).toHaveValue('Andrea Pirlo');
  await field.press('Enter');

  await expect(page.locator('.tab', { hasText: 'Andrea Pirlo' })).toBeVisible();
  await expect
    .poll(async () => ((await (await ctx.get(`${base}/api/v1/sessions/${ids.Pirlo}`)).json()) as { title: string }).title)
    .toBe('Andrea Pirlo');
});

test('Enter on a focused tab × asks to close the session', async ({ page }) => {
  const tab = await openTab(page, 'Pirlo');
  await tab.locator('.tab-close').focus();
  await page.keyboard.press('Enter');
  const dialog = page.getByRole('dialog', { name: 'Close session?' });
  await expect(dialog).toBeVisible();
  await dialog.getByRole('button', { name: 'Cancel' }).click();
  await expect(dialog).toHaveCount(0);
  await expect(tab).toBeVisible();
});
