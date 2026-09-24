import { test, expect, type APIRequestContext, type Page } from '@playwright/test';
import { apiCtx, seedWorkspace } from './seed';
import { expectFullyInViewport, expectNoHorizontalOverflow } from './helpers';

// ─────────────────────────────────────────────────────────────────────────────
// Site Studio (desktop-browser) — against the REAL /design/* routes, so it needs
// a daemon built from this tree (OTTO_E2E_BIN=<repo>/target/debug/ottod).
//
//   • a blank site opens on the six starter templates; picking Landing page
//     renders it on the canvas; Save makes v2;
//   • Layers selects the hero, the inspector edits its headline live, a
//     double-click edits text in place, the floating section toolbar and the
//     Publish menu stay fully inside the viewport, the mobile breakpoint
//     narrows the frame;
//   • the block library inserts a section (search → click);
//   • a zip export is a real archive whose site.css starts with the brand
//     tokens, and it (plus a local preview) is recorded with a pinned set;
//     the loopback preview renders the page.
// ─────────────────────────────────────────────────────────────────────────────

// The flows build on each other (template → edits → export).
test.describe.configure({ mode: 'serial' });

const V1 = '/api/v1';
let wsId = '';
let kitId = '';
let blankId = '';
const stamp = Date.now().toString(36);

async function postJson(ctx: APIRequestContext, url: string, data: unknown): Promise<any> {
  const r = await ctx.post(url, { data });
  expect(r.ok(), `${url} → ${r.status()} ${await r.text()}`).toBeTruthy();
  return r.json();
}

test.beforeAll(async () => {
  const { ctx, base } = await apiCtx();
  wsId = await seedWorkspace(ctx, base);
  const kit = await postJson(ctx, `${base}${V1}/design/artifacts`, {
    workspace_id: wsId,
    format: 'otto-brand',
    studio: 'brand',
    title: `Teal kit ${stamp}`,
    content: JSON.stringify({
      $schema: 'otto-brand/1',
      name: 'Teal kit',
      color: {
        primary: { $value: '#0F766E' },
        accent: { $value: '#F97360' },
        ink: { $value: '#10201F' },
        surface: { $value: '#FFFFFF' },
        'surface-alt': { $value: '#F2FAF8' },
      },
      radius: { md: { $value: 12 } },
    }),
  });
  kitId = kit.artifact.id;
  const project = await postJson(ctx, `${base}${V1}/design/projects`, { workspace_id: wsId, name: `Site project ${stamp}`, brand_kit_id: kitId });
  const blank = await postJson(ctx, `${base}${V1}/design/artifacts`, {
    workspace_id: wsId,
    project_id: project.id,
    format: 'otto-site',
    studio: 'site',
    title: `Launch site ${stamp}`,
    content: JSON.stringify({ type: 'otto-site', version: 1, title: 'Launch site', pages: [] }),
  });
  blankId = blank.artifact.id;
  await ctx.dispose();
});

test.beforeEach(async ({ page }) => {
  await page.addInitScript((id) => {
    localStorage.setItem('otto_workspace', id as string);
  }, wsId);
});

async function openSite(page: Page, id: string): Promise<void> {
  await page.goto(`/#/design/a/${id}`);
  await expect(page.locator('.shell')).toBeVisible({ timeout: 30_000 });
}

test('a blank site starts from a template and saves as a new version', async ({ page }) => {
  await openSite(page, blankId);
  await expect(page.getByTestId('site-templates')).toBeVisible();
  await expect(page.getByTestId('site-template')).toHaveCount(6);
  await page.getByTestId('site-template').filter({ hasText: 'Landing page' }).click();

  const canvas = page.getByTestId('site-page');
  await expect(canvas).toContainText('Every purchase moves you up.');
  await expect(page.getByTestId('design-dirty')).toBeVisible();
  await page.getByTestId('design-save').click();
  await expect(page.getByTestId('design-version-chip')).toHaveCount(2);
  await expect(page.getByTestId('design-dirty')).toHaveCount(0);
  await expectNoHorizontalOverflow(page);
});

test('edit a section from the inspector and in place; toolbars stay on screen', async ({ page }) => {
  await openSite(page, blankId);
  await expect(page.getByTestId('site-studio')).toBeVisible();
  await page.getByTestId('site-tab-layers').click();
  await page.getByTestId('site-layer').filter({ hasText: 'Hero' }).click();

  // The floating section toolbar is clamped inside the viewport.
  const toolbar = page.getByTestId('site-section-toolbar');
  await expectFullyInViewport(page, toolbar, 'section toolbar');

  // Inspector → canvas, live.
  const inspector = page.getByTestId('site-inspector');
  await inspector.getByLabel('Headline').fill(`Loyalty that feels like a gift ${stamp}`);
  await expect(page.getByTestId('site-page')).toContainText(`Loyalty that feels like a gift ${stamp}`);

  // Double-click text on the canvas edits it in place.
  const eyebrow = page.locator('[data-os-section="hero"] .os-eyebrow [data-os-edit="eyebrow"]');
  await eyebrow.dblclick();
  await page.keyboard.press('ControlOrMeta+A');
  await page.keyboard.type('Just launched');
  await page.keyboard.press('Enter');
  await expect(inspector.getByLabel('Eyebrow')).toHaveValue('Just launched');

  // Brand swatches come from the linked kit; a raw hex is flagged off-brand.
  await expect(inspector).toContainText('from Teal kit');
  await inspector.getByLabel('Custom colour (hex)').fill('#123456');
  await inspector.getByLabel('Custom colour (hex)').blur();
  await expect(inspector).toContainText('Off-brand colour');

  // Mobile breakpoint narrows the frame; Publish menu stays on screen.
  await page.getByTestId('site-bp-mobile').click();
  await expect(page.getByTestId('site-canvas')).toContainText('390 px');
  await page.getByTestId('site-publish').click();
  const menu = page.locator('.ctx-menu');
  await page.waitForTimeout(100);
  await expectFullyInViewport(page, menu, 'publish menu');
  await expect(menu.getByRole('menuitem', { name: /claude\.ai artifact/ })).toBeDisabled();
  await page.keyboard.press('Escape');

  await page.getByTestId('design-save').click();
  await expect(page.getByTestId('design-dirty')).toHaveCount(0);
});

test('the block library inserts a section', async ({ page }) => {
  await openSite(page, blankId);
  await page.getByTestId('site-tab-blocks').click();
  await page.getByTestId('site-block-search').fill('stats');
  const tile = page.getByTestId('site-block-tile').filter({ hasText: 'big numbers' });
  await expect(tile).toHaveCount(1);
  const before = await page.locator('[data-os-section]').count();
  await tile.click();
  await expect(page.locator('[data-os-section]')).toHaveCount(before + 1);
  await expect(page.getByTestId('site-page')).toContainText('The numbers speak.');
});

test('zip export and local preview are recorded with a pinned set', async () => {
  const { ctx, base } = await apiCtx();
  const zip = await ctx.post(`${base}${V1}/design/artifacts/${blankId}/export`, { data: { target: 'zip' } });
  expect(zip.status(), await zip.text().catch(() => '')).toBe(200);
  expect(zip.headers()['content-type']).toBe('application/zip');
  const bytes = await zip.body();
  expect(bytes.subarray(0, 4)).toEqual(Buffer.from([0x50, 0x4b, 0x03, 0x04]));
  expect(bytes.includes(Buffer.from('--brand-color-primary: #0F766E;'))).toBeTruthy();
  expect(bytes.includes(Buffer.from('index.html'))).toBeTruthy();

  const local = await postJson(ctx, `${base}${V1}/design/artifacts/${blankId}/export`, { target: 'local' });
  expect(local.url).toContain(`/design/artifacts/${blankId}/preview?publish=`);
  const html = await ctx.get(`${base}${local.url}`);
  expect(html.status()).toBe(200);
  expect(html.headers()['content-security-policy']).toContain('sandbox');
  expect(await html.text()).toContain('class="os-site"');

  const pubs = await (await ctx.get(`${base}${V1}/design/artifacts/${blankId}/publishes`)).json();
  expect(pubs.map((p: any) => p.target).sort()).toEqual(['local', 'zip']);
  expect(pubs[0].pinned_set[0].role).toBe('site');
  expect(pubs[0].pinned_set.some((r: any) => r.role === 'brand' && r.artifact_id === kitId)).toBeTruthy();

  const bad = await ctx.post(`${base}${V1}/design/artifacts/${blankId}/export`, { data: { target: 'claude_artifact' } });
  expect(bad.status()).toBe(400);
  await ctx.dispose();
});
