import { test, expect, type APIRequestContext, type Page } from '@playwright/test';
import { apiCtx, seedWorkspace } from './seed';
import { expectFullyInViewport, expectNoHorizontalOverflow } from './helpers';
import { STARTER_KITS } from '../src/modules/design-hall/brand/starters.ts';

// ─────────────────────────────────────────────────────────────────────────────
// Brand Kit (desktop-browser) — against the REAL /design/* + /brand routes, so
// it needs a daemon built from this tree (OTTO_E2E_BIN=<repo>/target/debug/ottod).
//
//   • a kit opens with its colour cards, live contrast badges, the "Applied to"
//     preview and "Used in" (a site naming token:color.primary + a frame linked
//     explicitly that only uses var(--brand-radius-md));
//   • trying a teal primary re-tints the preview and the banner says the change
//     reaches 1 design in 1 studio (the frame doesn't use the primary);
//   • "Save as v2" shows the impact preview first, saves, then "Approve v2" is an
//     explicit, confirmed step;
//   • an invalid hex blocks saving with the problem named;
//   • #/design/brand opens the most recent kit; a new kit starts from a starter;
//   • the Export menu stays inside the viewport and the CSS export uses the
//     contract names.
// ─────────────────────────────────────────────────────────────────────────────

const V1 = '/api/v1';
const stamp = Date.now().toString(36);
const KIT_TITLE = `Acme brand ${stamp}`;
let wsId = '';
let kitId = '';
let siteId = '';
let frameId = '';

async function postJson(ctx: APIRequestContext, url: string, data: unknown): Promise<any> {
  const r = await ctx.post(url, { data });
  expect(r.ok(), `${url} → ${r.status()} ${await r.text()}`).toBeTruthy();
  return r.json();
}

test.beforeAll(async () => {
  const { ctx, base } = await apiCtx();
  wsId = await seedWorkspace(ctx, base);
  const kitDoc = STARTER_KITS[0].build(KIT_TITLE);
  const kit = await postJson(ctx, `${base}${V1}/design/artifacts`, {
    workspace_id: wsId,
    studio: 'brand',
    format: 'otto-brand',
    title: KIT_TITLE,
    content: JSON.stringify(kitDoc, null, 2),
  });
  kitId = kit.artifact.id;
  // Approve v1 so consumers have something to follow.
  await postJson(ctx, `${base}${V1}/design/artifacts/${kitId}/approve`, {});

  // A site that names the primary + a radius (its `brand` key → uses_tokens).
  const site = await postJson(ctx, `${base}${V1}/design/artifacts`, {
    workspace_id: wsId,
    format: 'otto-site',
    title: `Rewards landing ${stamp}`,
    content: JSON.stringify({
      type: 'otto-site',
      brand: `otto://design/${kitId}`,
      pages: [{ id: 'home', sections: [{ id: 'hero', props: { bg: 'token:color.primary', r: 'token:radius.md' } }] }],
    }),
  });
  siteId = site.artifact.id;
  // A frame that only uses the radius, linked explicitly.
  const frame = await postJson(ctx, `${base}${V1}/design/artifacts`, {
    workspace_id: wsId,
    format: 'html',
    studio: 'frames',
    title: `Tier card ${stamp}`,
    content: '<!doctype html><div style="border-radius:var(--brand-radius-md)">Gold</div>',
  });
  frameId = frame.artifact.id;
  await postJson(ctx, `${base}${V1}/design/artifacts/${frameId}/links`, { rel: 'uses_tokens', dst_kind: 'artifact', dst_id: kitId });
  await ctx.dispose();
});

test.beforeEach(async ({ page }) => {
  await page.addInitScript((id) => {
    localStorage.setItem('otto_workspace', id as string);
  }, wsId);
});

async function openKit(page: Page, id = kitId): Promise<void> {
  await page.goto(`/#/design/brand/${id}`);
  await expect(page.locator('.shell')).toBeVisible({ timeout: 30_000 });
  await expect(page.getByTestId('brand-editor')).toBeVisible();
}

test('a kit shows colours with contrast, the live preview and where it is used', async ({ page }) => {
  await openKit(page);
  const colors = page.getByTestId('brand-color');
  await expect(colors).toHaveCount(6);
  const primary = page.locator('[data-testid="brand-color"][data-token="primary"]');
  await expect(primary.getByRole('textbox', { name: 'Primary hex value' })).toHaveValue('#5B3DF5');
  await expect(primary).toContainText('on white 6.1');
  await expect(primary).toContainText('--brand-color-primary');
  await expect(page.getByTestId('brand-applied')).toBeVisible();

  const used = page.getByTestId('brand-used-in');
  await expect(used).toContainText('2 designs');
  await expect(page.getByTestId('brand-used-row')).toHaveCount(2);
  await expect(page.getByTestId('brand-used-btn')).toContainText('Used in 2');
  await expectNoHorizontalOverflow(page);

  // The Export menu is clamped inside the viewport.
  await page.getByTestId('brand-exports').click();
  await page.waitForTimeout(100);
  await expectFullyInViewport(page, page.locator('.ctx-menu'), 'export menu');
  await page.keyboard.press('Escape');
});

test('a new primary previews its impact, saves as v2 and rolls out on approve', async ({ page }) => {
  await openKit(page);
  const applied = page.getByTestId('brand-applied');
  const before = await applied.evaluate((el) => getComputedStyle(el).getPropertyValue('--pv-primary').trim());

  await page.getByRole('button', { name: /Set primary to Teal/ }).click();
  await expect.poll(async () => applied.evaluate((el) => getComputedStyle(el).getPropertyValue('--pv-primary').trim())).not.toBe(before);
  const banner = page.getByTestId('brand-banner');
  await expect(banner).toContainText('affects 1 artifact in 1 studio', { timeout: 15_000 });

  // Save → the impact preview first (designs use this kit).
  await page.getByTestId('brand-save').click();
  const modal = page.getByTestId('brand-impact-modal');
  await expect(modal).toBeVisible();
  await expect(modal).toContainText('color.primary');
  await expect(modal).toContainText(`Rewards landing ${stamp}`);
  await expect(modal).not.toContainText(`Tier card ${stamp}`);
  await page.getByTestId('brand-impact-save').click();
  await expect(modal).toHaveCount(0);
  await expect(banner).toHaveCount(0);

  // v2 is saved but not approved → approving is explicit and confirmed.
  await expect(page.getByTestId('brand-approval')).toContainText('v2 is saved but not approved');
  await page.getByTestId('brand-approve').click();
  const confirm = page.getByRole('dialog', { name: 'Approve brand kit' });
  await expect(confirm).toContainText('switch to v2');
  await confirm.getByRole('button', { name: 'Approve v2' }).click();
  await expect(page.getByTestId('brand-approval')).toHaveCount(0);

  // The server agrees: v2 approved, the CSS export carries the new primary.
  const { ctx, base } = await apiCtx();
  const d = await (await ctx.get(`${base}${V1}/design/artifacts/${kitId}`)).json();
  expect(d.artifact.status).toBe('approved');
  expect(d.approved.seq).toBe(2);
  const css = await (await ctx.get(`${base}${V1}/design/artifacts/${kitId}/brand/export?format=css`)).text();
  expect(css).toContain('--brand-color-primary: #0F9D8A;');
  await ctx.dispose();
});

test('an invalid hex blocks saving and names the problem', async ({ page }) => {
  await openKit(page);
  const hex = page.getByRole('textbox', { name: 'Accent hex value' });
  await hex.fill('#12');
  await expect(page.getByTestId('brand-issues')).toContainText('color.accent');
  await expect(page.getByTestId('brand-save')).toBeDisabled();
  await hex.fill('#FFB547');
  await expect(page.getByTestId('brand-issues')).toHaveCount(0);
});

test('the Brand Kit page opens the latest kit and creates new ones from a starter', async ({ page }) => {
  await page.goto('/#/design/brand');
  await expect(page.locator('.shell')).toBeVisible({ timeout: 30_000 });
  await expect(page).toHaveURL(/#\/design\/brand\/[^/]+$/);
  await expect(page.getByTestId('brand-editor')).toBeVisible();

  await page.getByTestId('brand-switcher').click();
  await page.locator('.ctx-menu').getByRole('menuitem', { name: 'New brand kit…' }).click();
  const sheet = page.getByTestId('brand-new-modal');
  await expect(sheet).toBeVisible();
  await sheet.getByPlaceholder('Acme brand').fill(`Editorial ${stamp}`);
  await sheet.getByText('Editorial', { exact: true }).click();
  await page.getByTestId('brand-new-create').click();
  await expect(page.getByTestId('brand-switcher')).toContainText(`Editorial ${stamp}`);
  await expect(page.getByRole('textbox', { name: 'Primary hex value' })).toHaveValue('#0F766E');
});
