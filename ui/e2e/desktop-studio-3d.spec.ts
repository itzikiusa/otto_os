import { test, expect, type APIRequestContext, type Page } from '@playwright/test';
import { apiCtx, seedWorkspace } from './seed';
import { expectFullyInViewport, expectNoHorizontalOverflow } from './helpers';

// ─────────────────────────────────────────────────────────────────────────────
// 3D Studio 1.5 (desktop-browser) — against the REAL /design/* routes, so it
// needs a daemon built from this tree (OTTO_E2E_BIN=<repo>/target/debug/ottod):
// the Rust scene3d validator must accept v2 documents.
//
//   • a scene3d v2 design opens the studio layout: hierarchy + environment +
//     views, the viewport with its states bar and stats, Inspector / Links;
//   • a material preset and a brand swatch edit the document (token colour +
//     the kit's `brand` URI → a `uses_tokens` link after Save);
//   • picking a non-default state routes transform edits to that state;
//   • ✨ Generate, Export and the Text → 3D provider picker stay inside the
//     viewport; cloud providers are disabled without a key;
//   • Save view → a named camera in the document; Optimize for web reports a
//     size against the web budget;
//   • the daemon refuses an invalid v2 document (token colour in a v1 doc).
// ─────────────────────────────────────────────────────────────────────────────

const V1 = '/api/v1';
const stamp = Date.now().toString(36);
let wsId = '';
let sceneId = '';
let kitId = '';

async function postJson(ctx: APIRequestContext, url: string, data: unknown): Promise<any> {
  const r = await ctx.post(url, { data });
  expect(r.ok(), `${url} → ${r.status()} ${await r.text()}`).toBeTruthy();
  return r.json();
}

const SCENE = {
  type: 'otto-scene3d',
  version: 2,
  background: '#eceaf3',
  grid: true,
  environment: { preset: 'studio-soft', intensity: 1, background: true },
  camera: { position: [3.4, 2.1, 5.2], target: [0, 0.8, 0], fov: 30 },
  lights: [{ id: 'key', name: 'Key light', type: 'directional', position: [4, 6, 4], target: [0, 0.8, 0], intensity: 1.6, shadow: true }],
  objects: [
    { id: 'card', name: 'Card', type: 'box', radius: 0.05, position: [0, 0.95, 0], rotation: [0, 20, 0], scale: [1.6, 1, 0.03], material: { preset: 'glossy-plastic', color: '#5b3df5' } },
    { id: 'plinth', name: 'Plinth', type: 'cylinder', position: [0, 0.2, 0], rotation: [0, 0, 0], scale: [1.7, 0.4, 1.7], material: { preset: 'matte-paper' } },
  ],
  groups: [],
  states: [
    { id: 'idle', name: 'Idle' },
    { id: 'flipped', name: 'Flipped', duration_ms: 500, overrides: { card: { rotation: [0, 200, 0] } } },
  ],
  default_state: 'idle',
};

test.beforeAll(async () => {
  const { ctx, base } = await apiCtx();
  wsId = await seedWorkspace(ctx, base);
  const project = await postJson(ctx, `${base}${V1}/design/projects`, { workspace_id: wsId, name: `Rewards+ 3D ${stamp}` });
  const kit = await postJson(ctx, `${base}${V1}/design/artifacts`, {
    workspace_id: wsId,
    project_id: project.id,
    format: 'otto-brand',
    studio: 'brand',
    title: `Acme Brand Kit ${stamp}`,
    content: JSON.stringify({ $schema: 'otto-brand/1', name: 'Acme', color: { violet: { $value: '#5B3DF5' }, amber: { $value: '#F5A524' } } }),
  });
  kitId = kit.artifact.id;
  const scene = await postJson(ctx, `${base}${V1}/design/artifacts`, {
    workspace_id: wsId,
    project_id: project.id,
    format: 'scene3d',
    studio: '3d',
    title: `Rewards Card 3D ${stamp}`,
    content: JSON.stringify(SCENE, null, 2),
  });
  sceneId = scene.artifact.id;
  await ctx.dispose();
});

test.beforeEach(async ({ page }) => {
  await page.addInitScript((id) => localStorage.setItem('otto_workspace', id as string), wsId);
});

async function openScene(page: Page): Promise<void> {
  await page.goto(`/#/design/a/${sceneId}`);
  await expect(page.locator('.shell')).toBeVisible({ timeout: 30_000 });
  await expect(page.getByTestId('studio3d')).toBeVisible({ timeout: 30_000 });
  // The viewport booted three (the stats pill renders once it has).
  await expect(page.getByTestId('s3d-stats')).toContainText('2 objects', { timeout: 30_000 });
}

async function content(): Promise<any> {
  const { ctx, base } = await apiCtx();
  const r = await ctx.get(`${base}${V1}/design/artifacts/${sceneId}/content`);
  expect(r.ok()).toBeTruthy();
  const doc = JSON.parse(await r.text());
  await ctx.dispose();
  return doc;
}

function selectRow(page: Page, name: string) {
  return page.locator('.s3d-row').filter({ has: page.locator('.s3d-row-name', { hasText: new RegExp(`^${name}$`) }) });
}

test('the studio layout renders every region and has no horizontal overflow', async ({ page }) => {
  await openScene(page);
  await expect(page.getByTestId('s3d-left')).toContainText('Hierarchy');
  await expect(page.getByTestId('s3d-env')).toHaveValue('studio-soft');
  await expect(page.getByTestId('s3d-states')).toContainText('Flipped');
  await expect(page.getByTestId('s3d-tab-inspector')).toHaveAttribute('aria-selected', 'true');
  await expectNoHorizontalOverflow(page);
});

test('a preset and a brand swatch edit the document; Save links the kit', async ({ page }) => {
  await openScene(page);
  await selectRow(page, 'Card').click();
  await page.getByTestId('s3d-preset-brushed-metal').click();
  await expect(page.getByTestId('design-dirty')).toBeVisible();
  // The project's brand kit supplies the swatches.
  await page.getByRole('radio', { name: /Brand violet/ }).click();
  await page.getByTestId('design-save').click();
  await expect(page.getByTestId('design-dirty')).toHaveCount(0);

  const doc = await content();
  expect(doc.version).toBe(2);
  const card = doc.objects.find((o: any) => o.id === 'card');
  expect(card.material.preset).toBe('brushed-metal');
  expect(card.material.color).toBe('token:color.violet');
  expect(doc.brand).toContain(`otto://design/${kitId}`);

  // The kit is now a `uses_tokens` link ("Uses" on the Links tab).
  await page.getByTestId('design-tab-links').click();
  await expect(page.getByTestId('design-link-out').filter({ hasText: `Acme Brand Kit ${stamp}` })).toBeVisible();
});

test('a non-default state records transform edits as overrides', async ({ page }) => {
  await openScene(page);
  await selectRow(page, 'Card').click();
  await page.getByTestId('s3d-state-flipped').click();
  await expect(page.getByTestId('s3d-state-edit')).toContainText('Flipped');
  await page.getByTestId('s3d-state-idle').click();
  await expect(page.getByTestId('s3d-state-edit')).toHaveCount(0);
});

test('Generate, Export and the provider picker stay in the viewport; cloud is opt-in', async ({ page }) => {
  await openScene(page);
  await page.getByTestId('s3d-generate').click();
  const menu = page.locator('.ctx-menu');
  await page.waitForTimeout(100);
  await expectFullyInViewport(page, menu, 'Generate menu');
  await expect(menu.getByRole('menuitem', { name: 'Blockout from prompt (scene JSON)…' })).toBeVisible();
  await menu.getByRole('menuitem', { name: 'Text → 3D model…' }).click();

  const modal = page.getByTestId('gen3d-modal');
  await expect(modal).toBeVisible();
  await expect(modal.getByRole('radio', { name: /Cloud: Tripo/ })).toBeDisabled();
  await expect(modal.getByRole('radio', { name: /Cloud: Meshy/ })).toBeDisabled();
  await expect(modal).toContainText('sends your prompt');
  await expect(modal.getByRole('radio', { name: /Local: Otto \+ Blender MCP/ })).toBeChecked();
  await page.keyboard.press('Escape');

  await page.getByTestId('s3d-export').click();
  await page.waitForTimeout(100);
  await expectFullyInViewport(page, page.locator('.ctx-menu'), 'Export menu');
  await expect(page.locator('.ctx-menu').getByRole('menuitem', { name: /USDZ/ })).toBeVisible();
  await page.keyboard.press('Escape');
});

test('Save view adds a named camera; Optimize for web reports the budget', async ({ page }) => {
  await openScene(page);
  await page.getByRole('button', { name: 'Save the current view' }).click();
  const dlg = page.getByRole('dialog', { name: 'Save view' });
  await dlg.getByRole('textbox').fill('Hero angle');
  await dlg.getByRole('button', { name: 'Save view' }).click();
  await expect(page.getByTestId('s3d-left')).toContainText('Hero angle');
  await page.getByTestId('design-save').click();
  await expect(page.getByTestId('design-dirty')).toHaveCount(0);
  expect((await content()).cameras?.[0]?.id).toBe('hero-angle');

  await page.getByTestId('s3d-export').click();
  await page.locator('.ctx-menu').getByRole('menuitem', { name: /Optimize for web/ }).click();
  await expect(page.getByTestId('s3d-optimize')).toContainText('Web budget', { timeout: 60_000 });
});

test('the daemon validates scene3d v2 (a token colour needs version 2)', async () => {
  const { ctx, base } = await apiCtx();
  const bad = { ...SCENE, version: 1, objects: [{ ...SCENE.objects[0], material: { color: 'token:color.violet' } }] };
  const r = await ctx.put(`${base}${V1}/design/artifacts/${sceneId}/content`, { data: { content: JSON.stringify(bad) } });
  expect(r.status()).toBe(400);
  const src = { ...SCENE, objects: [{ id: 'gift', type: 'gltf', src: 'https://example.com/x.glb' }] };
  const r2 = await ctx.put(`${base}${V1}/design/artifacts/${sceneId}/content`, { data: { content: JSON.stringify(src) } });
  expect(r2.status()).toBe(400);
  await ctx.dispose();
});
