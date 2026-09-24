import { test, expect, type APIRequestContext } from '@playwright/test';
import { apiCtx, seedWorkspace } from './seed';
import { expectFullyInViewport, expectNoHorizontalOverflow, openApiEditor, openPage } from './helpers';

// ─────────────────────────────────────────────────────────────────────────────
// API client redesign (2026-09-24):
//   • an empty workspace opens on "Create your first request", not a blank editor
//   • the header's environment switcher is a clamped menu (many environments)
//     and switching updates the button and the {{variable}} line
//   • the variable line says what each {{var}} resolves to (value / not set)
//   • ⌘S on a new request opens the save sheet with a collection picker;
//     ⌘D duplicates into a new unsaved tab
//   • phone width: no horizontal page scroll, push navigation list ↔ editor
// Desktop-browser project only.
// ─────────────────────────────────────────────────────────────────────────────

let wsEmpty = '';
let wsFull = '';

async function post(ctx: APIRequestContext, url: string, data: unknown): Promise<any> {
  const r = await ctx.post(url, { data });
  if (!r.ok()) throw new Error(`POST ${url} → ${r.status()} ${await r.text()}`);
  return r.json();
}

test.beforeAll(async () => {
  const s = await apiCtx();
  wsEmpty = await seedWorkspace(s.ctx, s.base);
  wsFull = await seedWorkspace(s.ctx, s.base);
  const api = `${s.base}/api/v1/workspaces/${wsFull}/api-client`;
  const col = await post(s.ctx, `${api}/collections`, { name: 'Billing' });
  await post(s.ctx, `${api}/requests`, {
    collection_id: col.id, name: 'Get invoice', method: 'GET',
    url: '{{base_url}}/v1/invoices/{{invoice_id}}', headers: [], query: [], body_mode: 'none', body: '', auth: { type: 'none' },
  });
  // Enough environments to overflow the window: the switcher must clamp + scroll.
  for (let i = 0; i < 40; i++) {
    await post(s.ctx, `${api}/environments`, { name: `Env ${String(i).padStart(2, '0')}`, variables: { base_url: `https://env${i}.example.com` } });
  }
  await s.ctx.dispose().catch(() => {});
});

test.beforeEach(async ({ page }, testInfo) => {
  test.skip(testInfo.project.name !== 'desktop-browser', 'desktop-browser only');
  await page.addInitScript(() => localStorage.setItem('otto_rail_expanded', '0'));
});

test('an empty workspace opens on onboarding, and New request shows the editor', async ({ page }) => {
  await page.addInitScript((id) => localStorage.setItem('otto_workspace', id), wsEmpty);
  await openPage(page, 'api');
  await expect(page.getByText('Create your first request', { exact: true })).toBeVisible();
  await expect(page.getByLabel('Request URL')).toHaveCount(0);
  await page.getByRole('button', { name: 'New request', exact: true }).click();
  await expect(page.getByLabel('Request URL')).toBeFocused();
  await expect(page.locator('.req-tab')).toHaveCount(1);
});

test('environment switcher clamps into the window and drives {{variable}} values', async ({ page }) => {
  await page.setViewportSize({ width: 1280, height: 600 });
  await page.addInitScript((id) => localStorage.setItem('otto_workspace', id), wsFull);
  await openApiEditor(page);
  await page.locator('.req-open', { hasText: 'Get invoice' }).click();

  const vars = page.locator('.vars');
  await expect(vars.locator('.var-chip.missing')).toHaveCount(2); // no active env yet
  await page.getByRole('button', { name: /^Environment:/ }).click();
  const menu = page.locator('.ctx-menu');
  await page.waitForTimeout(100); // post-render clamp (rAF)
  await expectFullyInViewport(page, menu, 'environment menu');
  await menu.getByRole('menuitem', { name: 'Env 07' }).click();

  await expect(page.getByRole('button', { name: 'Environment: Env 07' })).toBeVisible();
  await expect(vars.locator('.var-chip', { hasText: 'base_url' })).toContainText('https://env7.example.com');
  await expect(vars.locator('.var-chip.missing')).toHaveCount(1);
  await expect(vars.locator('.var-chip.missing')).toContainText('invoice_id');
});

test('⌘S on a new request opens the save sheet; ⌘D duplicates into a new tab', async ({ page }) => {
  await page.addInitScript((id) => localStorage.setItem('otto_workspace', id), wsFull);
  await openApiEditor(page);
  await page.getByRole('button', { name: 'New request', exact: true }).click();
  await page.getByLabel('Request URL').fill('https://api.example.com/v1/ping');
  await page.keyboard.press('ControlOrMeta+s');
  const dlg = page.getByRole('dialog', { name: 'Save request' });
  await expect(dlg).toBeVisible();
  await dlg.getByLabel('Name').fill('Ping');
  await dlg.getByLabel('Save to').selectOption({ label: 'Billing' });
  await dlg.getByRole('button', { name: 'Save', exact: true }).click();
  await expect(page.locator('.req-open', { hasText: 'Ping' })).toBeVisible();
  await expect(page.locator('.where')).toContainText('Billing');

  const tabs = page.locator('.req-tab');
  const before = await tabs.count();
  await page.getByLabel('Request URL').click();
  await page.keyboard.press('ControlOrMeta+d');
  await expect(tabs).toHaveCount(before + 1);
  await expect(page.getByLabel('Request name')).toHaveValue('Ping copy');
  await expect(page.locator('.where')).toContainText('Not saved');
});

test('phone: no horizontal scroll, and the list is one tap away', async ({ page }) => {
  await page.setViewportSize({ width: 390, height: 844 });
  await page.addInitScript((id) => localStorage.setItem('otto_workspace', id), wsFull);
  await openApiEditor(page);
  await expectNoHorizontalOverflow(page);
  await page.getByRole('button', { name: 'Show saved requests' }).click();
  await expect(page.locator('.req-open', { hasText: 'Get invoice' })).toBeVisible();
  await expectNoHorizontalOverflow(page);
  await page.locator('.req-open', { hasText: 'Get invoice' }).click();
  await expect(page.getByLabel('Request URL')).toHaveValue('{{base_url}}/v1/invoices/{{invoice_id}}');
  await expectNoHorizontalOverflow(page);
});
