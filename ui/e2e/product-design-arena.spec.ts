import { test, expect, type Page } from '@playwright/test';
import { apiCtx, seedWorkspace } from './seed';
import { seedProductStory } from './seed-product';

// ── E2E: Design Arena (docs/design/product-design-arena.md §4) ─────────────────
//
// Seeds design artifacts through the API (an Excalidraw board, a scene3d
// document, a Mermaid diagram) and asserts the arena: grouped asset list, the
// Excalidraw React island renders for a board, the 3D scene opens with the
// Hierarchy pane listing objects + a status line, the code view's edit reaches
// `PUT /product/attachments/{aid}/content` (600 ms debounce) and the new bytes
// survive a reload, New ▾ → template creates an HTML screen that renders inside a
// SANDBOXED iframe inside a device frame, and the phone layout collapses to the
// segmented single pane.
test.use({ viewport: { width: 1280, height: 900 }, actionTimeout: 12_000 });

let workspaceId = '';
let storyId = '';
let mermaidId = '';
const STORY_TITLE = `E2E Arena ${Date.now()}-${Math.random().toString(36).slice(2, 8)}`;

const BOARD = JSON.stringify({
  type: 'excalidraw',
  version: 2,
  source: 'otto',
  elements: [
    { id: 'r1', type: 'rectangle', x: 0, y: 0, width: 200, height: 120, strokeColor: '#1e293b', backgroundColor: 'transparent', fillStyle: 'solid', strokeWidth: 1, roughness: 1, opacity: 100, angle: 0, seed: 1, version: 1, versionNonce: 1, isDeleted: false, groupIds: [], frameId: null, boundElements: [], updated: 1, link: null, locked: false, roundness: { type: 3 } },
  ],
  appState: { viewBackgroundColor: '#ffffff', gridSize: 8 },
  files: {},
});
const SCENE = JSON.stringify({
  type: 'otto-scene3d',
  version: 1,
  background: '#0f172a',
  grid: true,
  camera: { position: [6, 5, 8], target: [0, 1, 0], fov: 50 },
  lights: [
    { id: 'sun', type: 'directional', position: [5, 10, 5], intensity: 1.2, color: '#ffffff', shadow: true },
    { id: 'amb', type: 'ambient', intensity: 0.4 },
  ],
  objects: [
    { id: 'floor', name: 'Floor', type: 'plane', position: [0, 0, 0], rotation: [-90, 0, 0], scale: [20, 20, 1], material: { color: '#334155', roughness: 0.9 } },
    { id: 'crate', name: 'Crate', type: 'box', position: [0, 0.5, 0], rotation: [0, 30, 0], scale: [1, 1, 1], material: { color: '#f59e0b' } },
  ],
  groups: [{ id: 'props', name: 'Props', children: ['crate'] }],
});
const MERMAID = 'flowchart LR\n  A[Start] --> B[End]\n';
const EDIT_MARK = `E2E_EDIT_${Math.random().toString(36).slice(2, 8)}`;

const b64 = (s: string) => Buffer.from(s, 'utf8').toString('base64');

/** Upload a design artifact the way the arena does: `kind` follows Track A's
 *  convention — `mockup` for html/mermaid, `design` for excalidraw/scene3d. */
async function upload(filename: string, mime: string, body: string): Promise<string> {
  const { ctx, base } = await apiCtx();
  const kind = mime === 'text/html' || mime === 'text/vnd.mermaid' ? 'mockup' : 'design';
  const r = await ctx.post(`${base}/api/v1/product/stories/${storyId}/attachments`, {
    data: { filename, mime, kind, data_b64: b64(body) },
  });
  if (!r.ok()) throw new Error(`upload ${filename} → ${r.status()} ${await r.text()}`);
  const att = await r.json();
  await ctx.dispose();
  return att.id as string;
}

test.beforeAll(async () => {
  const { ctx, base } = await apiCtx();
  workspaceId = await seedWorkspace(ctx, base);
  const seeded = await seedProductStory(ctx, base, workspaceId);
  storyId = seeded.storyId;
  const r = await ctx.patch(`${base}/api/v1/product/stories/${storyId}/draft`, {
    data: { title: STORY_TITLE, body_md: `# ${STORY_TITLE}\n\nSeeded for the Design Arena E2E.` },
  });
  if (!r.ok()) throw new Error(`rename story → ${r.status()} ${await r.text()}`);
  await ctx.dispose();
  await upload('wireframes.excalidraw', 'application/vnd.excalidraw+json', BOARD);
  await upload('level.scene3d.json', 'application/vnd.otto.scene3d+json', SCENE);
  mermaidId = await upload('flow.mmd', 'text/vnd.mermaid', MERMAID);
});

test.beforeEach(async ({ page }) => {
  await page.addInitScript((wsId) => {
    localStorage.setItem('otto_workspace', wsId as string);
    localStorage.setItem('otto_rail_expanded', '0');
  }, workspaceId);
});

async function openArena(page: Page): Promise<void> {
  await page.goto('/#/product');
  await expect(page.locator('.product-page')).toBeVisible({ timeout: 30_000 });
  await page.waitForLoadState('networkidle').catch(() => {});
  const row = page.locator('.story-row', { hasText: STORY_TITLE }).first();
  await expect(row).toBeVisible({ timeout: 20_000 });
  await row.click();
  await expect(page.locator('.overview')).toBeVisible({ timeout: 20_000 });
  await page.getByRole('tab', { name: 'Story', exact: true }).click();
  await page.locator('.tab-strip .st', { hasText: 'Design' }).first().click();
  await expect(page.locator('.design-arena')).toBeVisible({ timeout: 15_000 });
}

test('arena: grouped assets; an Excalidraw board renders the React island', async ({ page }) => {
  test.setTimeout(90_000);
  await openArena(page);

  // Groups: Boards / Diagrams / 3D from the seeded rows.
  await expect(page.locator('.group-head', { hasText: 'Boards' })).toBeVisible({ timeout: 15_000 });
  await expect(page.locator('.group-head', { hasText: 'Diagrams' })).toBeVisible();
  await expect(page.locator('.group-head', { hasText: '3D' })).toBeVisible();

  await page.locator('.mockup-row', { hasText: 'wireframes.excalidraw' }).click();
  // The island mounted Excalidraw (its root class) inside our board host.
  await expect(page.locator('.design-board .excalidraw')).toBeVisible({ timeout: 30_000 });
  await expect(page.locator('.stage-status')).toContainText(/1 elements/);
  // Inspector shows the type; the Annotate toggle exists for boards.
  await expect(page.locator('.arena-inspector')).toContainText('Board');
  await expect(page.locator('.stage-toolbar .tb-btn', { hasText: 'Annotate' })).toBeVisible();
});

test('arena: a scene3d opens with the Hierarchy pane and an object count', async ({ page }) => {
  test.setTimeout(90_000);
  await openArena(page);
  await page.locator('.mockup-row', { hasText: 'level.scene3d.json' }).click();

  // The left pane flips to Hierarchy while a scene is open; the status line counts.
  const hierTab = page.locator('.pane-switch .ss', { hasText: 'Hierarchy' });
  await expect(hierTab).toBeVisible({ timeout: 15_000 });
  await expect(hierTab).toHaveClass(/active/);
  await expect(page.locator('.stage-status')).toContainText(/2 objects · 2 lights/);
  // The hierarchy names the objects (or, with Track C's placeholder, counts them).
  await expect(page.locator('.arena-assets .pane-body')).toContainText(/Floor|Crate|2 objects/);
  // Play toggle + Blender section in the inspector (installed or not, the
  // script download is always offered).
  await expect(page.locator('.stage-toolbar .tb-btn', { hasText: 'Play' })).toBeVisible();
  await expect(page.locator('.arena-inspector .p-btn', { hasText: 'Download script' })).toBeVisible({ timeout: 15_000 });
  // Back to the asset list via the switch.
  await page.locator('.pane-switch .ss', { hasText: 'Assets' }).click();
  await expect(page.locator('.mockup-row', { hasText: 'level.scene3d.json' })).toBeVisible();
});

test('arena: code-view edit autosaves via PUT …/content and survives a reload', async ({ page }) => {
  test.setTimeout(90_000);
  await openArena(page);
  await page.locator('.mockup-row', { hasText: 'flow.mmd' }).click();
  await expect(page.locator('.mockup-stage iframe.mockup-frame')).toBeVisible({ timeout: 20_000 });

  await page.locator('.stage-toolbar .tb-btn', { hasText: 'Source' }).click();
  const code = page.locator('.code-view');
  await expect(code).toBeVisible();
  await expect(code).toHaveValue(/flowchart LR/);

  // Type a new node line; the debounced PUT lands ≈600 ms after the last keystroke.
  const putP = page.waitForResponse(
    (r) => r.url().includes(`/product/attachments/${mermaidId}/content`) && r.request().method() === 'PUT',
  );
  await code.fill(`flowchart LR\n  A[Start] --> B[${EDIT_MARK}]\n`);
  const put = await putP;
  expect(put.ok(), `PUT content → ${put.status()}`).toBeTruthy();
  await expect(page.locator('.stage-status')).toContainText(/saved/, { timeout: 10_000 });

  // Reload → the server has the new bytes and the viewer shows them.
  await page.reload();
  await openArena(page);
  await page.locator('.mockup-row', { hasText: 'flow.mmd' }).click();
  await page.locator('.stage-toolbar .tb-btn', { hasText: 'Source' }).click();
  await expect(page.locator('.code-view')).toHaveValue(new RegExp(EDIT_MARK), { timeout: 15_000 });
  const { ctx, base } = await apiCtx();
  const body = await (await ctx.get(`${base}/api/v1/product/attachments/${mermaidId}`)).text();
  await ctx.dispose();
  expect(body).toContain(EDIT_MARK);
});

test('arena: New ▾ → template creates an HTML screen in a device frame (sandboxed iframe)', async ({ page }) => {
  test.setTimeout(90_000);
  await openArena(page);

  await page.locator('.arena-assets .p-btn', { hasText: 'New' }).first().click();
  const menu = page.locator('.ctx-menu');
  await expect(menu).toBeVisible();
  await menu.locator('.ctx-search-input').fill('Landing');
  await page.getByRole('menuitem', { name: 'Template: Landing page' }).click();

  const row = page.locator('.mockup-row', { hasText: 'landing.html' });
  await expect(row).toBeVisible({ timeout: 20_000 });
  await expect(row).toHaveClass(/active/);
  const frame = page.locator('.mockup-stage iframe.mockup-frame');
  await expect(frame).toBeVisible({ timeout: 20_000 });
  await expect(frame).toHaveAttribute('sandbox');

  // Device frame chooser: iPhone bezel wraps the iframe.
  await page.locator('.stage-toolbar .seg', { hasText: 'iPhone' }).click();
  await expect(page.locator('.device.iphone iframe.mockup-frame')).toBeVisible();
  await page.locator('.stage-toolbar .seg', { hasText: 'Fit' }).click();
  await expect(page.locator('.device')).toHaveCount(0);

  // Real persistence: the row survives a reload and the API lists it with the
  // template's bytes (not just local DOM state).
  await page.reload();
  await openArena(page);
  await expect(page.locator('.mockup-row', { hasText: 'landing.html' })).toBeVisible({ timeout: 20_000 });
  const { ctx, base } = await apiCtx();
  const atts = (await (await ctx.get(`${base}/api/v1/product/stories/${storyId}/attachments`)).json()) as Array<{
    id: string; filename: string; mime: string;
  }>;
  const landing = atts.find((a) => a.filename === 'landing.html');
  expect(landing, 'landing.html should be persisted').toBeTruthy();
  expect(landing!.mime).toBe('text/html');
  const body = await (await ctx.get(`${base}/api/v1/product/attachments/${landing!.id}`)).text();
  await ctx.dispose();
  expect(body).toContain('Withdrawals that settle before the kettle boils');
});

test('arena: ≤640px collapses to a segmented single pane', async ({ page }) => {
  test.setTimeout(60_000);
  await page.setViewportSize({ width: 390, height: 844 });
  await page.goto('/#/product');
  await expect(page.locator('.product-page')).toBeVisible({ timeout: 30_000 });
  await page.locator('.story-row', { hasText: STORY_TITLE }).first().click();
  await page.getByRole('tab', { name: 'Story', exact: true }).click();
  await page.locator('.tab-strip .st', { hasText: 'Design' }).first().click();
  const arena = page.locator('.design-arena');
  await expect(arena).toBeVisible({ timeout: 15_000 });
  const seg = arena.locator('.arena-seg');
  await expect(seg).toBeVisible();
  // Assets first; the stage/inspector are hidden until their segment is tapped.
  await expect(arena.locator('.arena-assets')).toBeVisible();
  await expect(arena.locator('.arena-stage')).toBeHidden();
  await arena.locator('.mockup-row', { hasText: 'flow.mmd' }).click();
  await expect(arena.locator('.arena-stage')).toBeVisible();
  await expect(arena.locator('.arena-assets')).toBeHidden();
  await seg.locator('.seg', { hasText: 'Inspector' }).click();
  await expect(arena.locator('.arena-inspector')).toBeVisible();
  await expect(arena.locator('.arena-stage')).toBeHidden();
  // No horizontal overflow on the phone.
  const over = await page.evaluate(() => document.documentElement.scrollWidth > document.documentElement.clientWidth + 1);
  expect(over).toBe(false);
});
