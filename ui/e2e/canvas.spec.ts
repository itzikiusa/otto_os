import { test, expect, type Page, type APIRequestContext } from '@playwright/test';
import { readFileSync, existsSync, readdirSync } from 'node:fs';
import { join } from 'node:path';
import { apiCtx, seedWorkspace } from './seed';

// ── E2E: Canvas Studio — TWO modes, file-backed ────────────────────────────────
//
// Excalidraw mode: the agent writes a per-scene `canvas.json`; the real Excalidraw
//   editor loads it and the user can add/remove shapes by hand (which save back).
// Mermaid mode: the agent writes a per-scene `canvas.mermaid`; Mermaid's own
//   renderer draws ANY type (flowchart / sequence / class / …).
// We drive the isolated daemon to prove both render, both persist to the real file
// ON DISK, and manual Excalidraw edits autosave.

test.use({ viewport: { width: 1360, height: 900 }, actionTimeout: 12_000 });

let workspaceId = '';

test.beforeAll(async () => {
  const { ctx, base } = await apiCtx();
  workspaceId = await seedWorkspace(ctx, base);
  await ctx.dispose();
});

test.beforeEach(async ({ page }) => {
  await page.addInitScript((wsId) => {
    localStorage.setItem('otto_workspace', wsId as string);
    localStorage.setItem('otto_rail_expanded', '0');
  }, workspaceId);
});

// -- on-disk file helpers (the isolated daemon's data dir from daemon.json) --------
function canvasFiles(name: string): string[] {
  const slot = process.env.OTTO_E2E_SLOT ?? '0';
  try {
    const meta = JSON.parse(
      readFileSync(join(process.cwd(), 'e2e', `.auth-${slot}`, 'daemon.json'), 'utf8'),
    ) as { dataDir: string };
    const base = join(meta.dataDir, 'canvas');
    if (!existsSync(base)) return [];
    return readdirSync(base)
      .map((d) => join(base, d, name))
      .filter((f) => existsSync(f));
  } catch {
    return [];
  }
}

async function seedScene(
  ctx: APIRequestContext,
  base: string,
  title: string,
  format: 'mermaid' | 'excalidraw',
  source: string,
  opts?: { section?: string; story_id?: string },
): Promise<string> {
  const doc = { type: 'otto-canvas', version: 1, format, source };
  const r = await ctx.post(`${base}/api/v1/workspaces/${workspaceId}/canvas/scenes`, {
    data: { title, doc, section: opts?.section, story_id: opts?.story_id },
  });
  expect(r.ok()).toBeTruthy();
  return ((await r.json()) as { id: string }).id;
}

async function openCanvas(page: Page): Promise<void> {
  await page.goto('/#/canvas');
  await expect(page.locator('.canvas-page')).toBeVisible({ timeout: 30_000 });
}
async function openScene(page: Page, title: string): Promise<void> {
  await openCanvas(page);
  await page.locator('.scene-list .row', { hasText: title }).first().click();
}

async function askAi(page: Page, prompt: string): Promise<void> {
  await page.getByRole('button', { name: /ask ai/i }).click();
  await page.getByPlaceholder(/ask for a diagram or a change/i).fill(prompt);
  await page.getByRole('button', { name: 'Send' }).click();
}

// ---------------------------------------------------------------------------
// Mode selector + create
// ---------------------------------------------------------------------------

test('the hero offers both modes; Excalidraw board mounts on create', async ({ page }) => {
  test.setTimeout(90_000);
  await openCanvas(page);
  await expect(page.getByRole('button', { name: /Excalidraw board/i })).toBeVisible({
    timeout: 30_000,
  });
  await expect(page.getByRole('button', { name: /Mermaid diagram/i })).toBeVisible();
  await page.getByRole('button', { name: /Excalidraw board/i }).click();
  await expect(page.locator('.excali .excalidraw').first()).toBeVisible({ timeout: 30_000 });
});

// ---------------------------------------------------------------------------
// Excalidraw mode
// ---------------------------------------------------------------------------

const EXCALI_SCENE = JSON.stringify({
  type: 'excalidraw',
  elements: [
    { type: 'rectangle', id: 'a', x: 40, y: 40, width: 160, height: 60, backgroundColor: '#dcfce7', strokeColor: '#16a34a', label: { text: '🚀 Start' } },
    { type: 'diamond', id: 'b', x: 40, y: 160, width: 180, height: 80, backgroundColor: '#fef9c3', strokeColor: '#ca8a04', label: { text: '❓ Valid?' } },
    { type: 'rectangle', id: 'c', x: 300, y: 160, width: 160, height: 60, backgroundColor: '#eef2ff', strokeColor: '#6366f1', label: { text: '⚙️ Process' } },
    { type: 'arrow', id: 'e1', start: { id: 'a' }, end: { id: 'b' } },
    { type: 'arrow', id: 'e2', start: { id: 'b' }, end: { id: 'c' }, label: { text: 'yes' } },
  ],
});

test('Excalidraw mode: a seeded canvas.json renders in the real Excalidraw editor', async ({
  page,
}) => {
  test.setTimeout(90_000);
  const { ctx, base } = await apiCtx();
  await seedScene(ctx, base, 'Ex Seeded', 'excalidraw', EXCALI_SCENE);
  await ctx.dispose();

  await openScene(page, 'Ex Seeded');
  await expect(page.locator('.excali .excalidraw').first()).toBeVisible({ timeout: 30_000 });
  // Excalidraw drew the elements onto its own <canvas>.
  await expect(page.locator('.excalidraw canvas').first()).toBeVisible({ timeout: 20_000 });
});

test('Excalidraw mode: manual draw → autosave PUT updates canvas.json on disk', async ({
  page,
}) => {
  test.setTimeout(90_000);
  const { ctx, base } = await apiCtx();
  await seedScene(
    ctx,
    base,
    'Ex Edit',
    'excalidraw',
    JSON.stringify({ type: 'excalidraw', elements: [] }),
  );
  await ctx.dispose();

  await openScene(page, 'Ex Edit');
  const canvasEl = page.locator('.excalidraw canvas').first();
  await expect(canvasEl).toBeVisible({ timeout: 30_000 });
  const box = await canvasEl.boundingBox();
  expect(box).not.toBeNull();
  const b = box!;

  const saved = page.waitForResponse(
    (r) => r.request().method() === 'PUT' && /\/canvas\/scenes\//.test(r.url()),
    { timeout: 20_000 },
  );

  // Draw a rectangle (keyboard 'r' + drag) in a clear area.
  await page.mouse.click(b.x + b.width * 0.5, b.y + b.height * 0.5);
  await page.keyboard.press('r');
  const x1 = b.x + b.width * 0.38;
  const y1 = b.y + b.height * 0.4;
  await page.mouse.move(x1, y1);
  await page.mouse.down();
  await page.mouse.move(x1 + 180, y1 + 130, { steps: 18 });
  await page.mouse.up();

  // Excalidraw's styles panel (always has Opacity) appears → the shape landed.
  await expect(page.locator('.excalidraw').getByText(/opacity/i).first()).toBeVisible({
    timeout: 10_000,
  });
  // …and it autosaved the FULL Excalidraw scene back to the scene doc (the json
  // the agent also edits) — verify the PUT body carries the new rectangle.
  const resp = await saved;
  expect(resp.ok()).toBeTruthy();
  expect(resp.request().postData() ?? '').toContain('rectangle');
});

test('Excalidraw mode: Ask AI → the agent writes canvas.json (on disk) + renders', async ({
  page,
}) => {
  test.setTimeout(90_000);
  const { ctx, base } = await apiCtx();
  await seedScene(
    ctx,
    base,
    'Ex Ask',
    'excalidraw',
    JSON.stringify({ type: 'excalidraw', elements: [] }),
  );
  await ctx.dispose();

  await openScene(page, 'Ex Ask');
  await expect(page.locator('.excali .excalidraw').first()).toBeVisible({ timeout: 30_000 });

  await page.getByRole('button', { name: /ask ai/i }).click();
  await page.getByPlaceholder(/ask for a diagram or a change/i).fill('order flow with a decision');
  await page.getByRole('button', { name: 'Send' }).click();

  await expect(page.getByText(/drawn on canvas/i).first()).toBeVisible({ timeout: 30_000 });
  await expect
    .poll(() => canvasFiles('canvas.json').length, { timeout: 15_000 })
    .toBeGreaterThan(0);
  const file = canvasFiles('canvas.json').slice(-1)[0];
  expect(readFileSync(file, 'utf8')).toContain('excalidraw');
});

// ---------------------------------------------------------------------------
// Mermaid mode — every diagram type renders
// ---------------------------------------------------------------------------

const MERMAID = {
  flowchart: 'flowchart TD\n  A(["Start"]) --> B{"OK?"}\n  B -->|yes| C["Do"]\n  B -->|no| D["Stop"]',
  sequence:
    'sequenceDiagram\n  participant A as Client\n  participant B as Server\n  A->>B: request\n  B-->>A: response',
  class:
    'classDiagram\n  class Order {\n    +int id\n    +float total\n    +submit()\n  }\n  Order --> Customer',
};

for (const [kind, src] of Object.entries(MERMAID)) {
  test(`Mermaid mode: a ${kind} diagram renders`, async ({ page }) => {
    test.setTimeout(90_000);
    const { ctx, base } = await apiCtx();
    await seedScene(ctx, base, `Mmd ${kind}`, 'mermaid', src);
    await ctx.dispose();

    await openScene(page, `Mmd ${kind}`);
    // Mermaid's own SVG renders on the board.
    await expect(page.locator('.board svg').first()).toBeVisible({ timeout: 25_000 });
  });
}

test('Mermaid mode: Ask AI → the agent writes canvas.mermaid (on disk) + renders', async ({
  page,
}) => {
  test.setTimeout(90_000);
  const { ctx, base } = await apiCtx();
  await seedScene(ctx, base, 'Mmd Ask', 'mermaid', '');
  await ctx.dispose();

  await openScene(page, 'Mmd Ask');
  await expect(page.locator('.board').first()).toBeVisible({ timeout: 30_000 });

  await page.getByRole('button', { name: /ask ai/i }).click();
  await page.getByPlaceholder(/ask for a diagram or a change/i).fill('order flow');
  await page.getByRole('button', { name: 'Send' }).click();

  await expect(page.getByText(/drawn on canvas/i).first()).toBeVisible({ timeout: 30_000 });
  await expect(page.locator('.board svg').first()).toBeVisible({ timeout: 25_000 });
  await expect
    .poll(() => canvasFiles('canvas.mermaid').length, { timeout: 15_000 })
    .toBeGreaterThan(0);
  const file = canvasFiles('canvas.mermaid').slice(-1)[0];
  expect(readFileSync(file, 'utf8')).toContain('flowchart');
});

// ---------------------------------------------------------------------------
// Scenes + phone
// ---------------------------------------------------------------------------

test('handles multiple scenes — list + switch', async ({ page }) => {
  test.setTimeout(90_000);
  const { ctx, base } = await apiCtx();
  await seedScene(ctx, base, 'Multi A', 'excalidraw', EXCALI_SCENE);
  await seedScene(ctx, base, 'Multi B', 'mermaid', MERMAID.flowchart);
  await ctx.dispose();

  await openScene(page, 'Multi B');
  await expect(page.locator('.board svg').first()).toBeVisible({ timeout: 25_000 });
  await page.locator('.scene-list .row', { hasText: 'Multi A' }).first().click();
  await expect(page.locator('.excali .excalidraw').first()).toBeVisible({ timeout: 30_000 });
});

// ---------------------------------------------------------------------------
// Sections, rename, provider, Product link
// ---------------------------------------------------------------------------

test('sections: scenes group under a collapsible section header', async ({ page }) => {
  test.setTimeout(90_000);
  const { ctx, base } = await apiCtx();
  await seedScene(ctx, base, 'Root One', 'excalidraw', EXCALI_SCENE);
  await seedScene(ctx, base, 'In Lane', 'excalidraw', EXCALI_SCENE, {
    section: 'Platform/Staging',
  });
  await ctx.dispose();

  await openCanvas(page);
  // The section header renders with the path ("Platform / Staging").
  await expect(page.locator('.scene-list .section-head', { hasText: /Platform \/ Staging/ })).toBeVisible(
    { timeout: 20_000 },
  );
  // The sectioned scene is nested.
  await expect(page.locator('.scene-list .row.nested', { hasText: 'In Lane' })).toBeVisible();
});

test('rename: inline rename updates the scene title', async ({ page }) => {
  test.setTimeout(90_000);
  const { ctx, base } = await apiCtx();
  await seedScene(ctx, base, 'Before Rename', 'excalidraw', EXCALI_SCENE);
  await ctx.dispose();

  await openCanvas(page);
  const row = page.locator('.scene-list .row', { hasText: 'Before Rename' }).first();
  await row.hover();
  await row.getByRole('button', { name: 'Rename' }).click();
  // The shared prompt dialog appears — type the new name + confirm.
  const input = page.locator('.cf-input, dialog input, [role="dialog"] input').first();
  await input.fill('After Rename');
  await page.getByRole('button', { name: 'Rename', exact: true }).last().click();
  await expect(page.locator('.scene-list .row', { hasText: 'After Rename' })).toBeVisible({
    timeout: 15_000,
  });
});

test('provider: changing the agent persists (PUT) on the scene', async ({ page }) => {
  test.setTimeout(90_000);
  const { ctx, base } = await apiCtx();
  await seedScene(ctx, base, 'Provider Scene', 'excalidraw', EXCALI_SCENE);
  await ctx.dispose();

  await openScene(page, 'Provider Scene');
  await expect(page.locator('.excali .excalidraw').first()).toBeVisible({ timeout: 30_000 });
  await page.getByRole('button', { name: /ask ai/i }).click();
  const select = page.locator('.assistant select.provider');
  // Only present when >1 provider is available.
  if (await select.count()) {
    const saved = page.waitForResponse(
      (r) => r.request().method() === 'PUT' && /\/canvas\/scenes\//.test(r.url()),
      { timeout: 15_000 },
    );
    await select.selectOption('codex');
    const resp = await saved;
    expect(resp.ok()).toBeTruthy();
  }
});

test('product link: linked-canvases endpoint returns a story-linked canvas', async ({ request }) => {
  test.setTimeout(60_000);
  const { ctx, base } = await apiCtx();
  // Create a product story (draft), then a canvas linked to it.
  const sr = await ctx.post(`${base}/api/v1/workspaces/${workspaceId}/product/drafts`, {
    data: { title: 'Linked Story' },
  });
  expect(sr.ok()).toBeTruthy();
  const detail = (await sr.json()) as { id?: string; story?: { id: string } };
  const storyId = detail.story?.id ?? detail.id ?? '';
  expect(storyId).not.toBe('');
  const sceneId = await seedScene(ctx, base, 'Story Canvas', 'excalidraw', EXCALI_SCENE, {
    story_id: storyId,
  });
  const linked = await ctx.get(`${base}/api/v1/product/stories/${storyId}/linked-canvases`);
  expect(linked.ok()).toBeTruthy();
  const list = (await linked.json()) as { id: string }[];
  expect(list.some((c) => c.id === sceneId)).toBeTruthy();
  await ctx.dispose();
  void request;
});

test('product link: linking an EXISTING canvas attaches it to the story', async ({ request }) => {
  test.setTimeout(60_000);
  const { ctx, base } = await apiCtx();
  const sr = await ctx.post(`${base}/api/v1/workspaces/${workspaceId}/product/drafts`, {
    data: { title: 'Relink Story' },
  });
  expect(sr.ok()).toBeTruthy();
  const detail = (await sr.json()) as { id?: string; story?: { id: string } };
  const storyId = detail.story?.id ?? detail.id ?? '';
  // A canvas with NO story link.
  const sceneId = await seedScene(ctx, base, 'Free Canvas', 'excalidraw', EXCALI_SCENE);
  // Link it to the story (the reverse direction: PUT story_id).
  const put = await ctx.put(`${base}/api/v1/canvas/scenes/${sceneId}`, {
    data: { story_id: storyId },
  });
  expect(put.ok()).toBeTruthy();
  const linked = await ctx.get(`${base}/api/v1/product/stories/${storyId}/linked-canvases`);
  const list = (await linked.json()) as { id: string }[];
  expect(list.some((c) => c.id === sceneId)).toBeTruthy();
  await ctx.dispose();
  void request;
});

test('Excalidraw: the REAL agent scene renders (no crash / no scatter build)', async ({ page }) => {
  test.setTimeout(90_000);
  const { ctx, base } = await apiCtx();
  // The actual agent-written simplified scene (41 shapes + 32 arrows + lanes).
  const source = readFileSync(join(process.cwd(), 'e2e', '_agent_scene.json'), 'utf8');
  await seedScene(ctx, base, 'Real Agent', 'excalidraw', source);
  await ctx.dispose();

  await openScene(page, 'Real Agent');
  // The builder turns it into a valid Excalidraw scene that mounts + draws.
  await expect(page.locator('.excalidraw canvas').first()).toBeVisible({ timeout: 30_000 });
  // Excalidraw's styles/menu chrome present ⇒ the scene loaded (not an error page).
  await expect(page.locator('.excalidraw [aria-label*="Rectangle" i]').first()).toBeVisible({
    timeout: 20_000,
  });
});

test('New scene: the format menu lets you pick Mermaid or Excalidraw', async ({ page }) => {
  test.setTimeout(90_000);
  const { ctx, base } = await apiCtx();
  await seedScene(ctx, base, 'Existing', 'excalidraw', EXCALI_SCENE); // so the list (+menu) shows
  await ctx.dispose();

  await openCanvas(page);
  await page.getByRole('button', { name: /New scene/i }).click();
  // The menu (scoped, so it doesn't collide with the hero's mode cards).
  const menu = page.locator('.scene-list .new-menu');
  await expect(menu.getByRole('button', { name: /Excalidraw board/i })).toBeVisible({
    timeout: 10_000,
  });
  await menu.getByRole('button', { name: /Mermaid diagram/i }).click();
  // A fresh Mermaid board mounts.
  await expect(page.locator('.board').first()).toBeVisible({ timeout: 30_000 });
});

test('Mermaid: the Code editor edits the SAME .mermaid file + live-previews', async ({ page }) => {
  test.setTimeout(90_000);
  const { ctx, base } = await apiCtx();
  await seedScene(ctx, base, 'CodeEdit', 'mermaid', MERMAID.flowchart);
  await ctx.dispose();

  await openScene(page, 'CodeEdit');
  await expect(page.locator('.board svg').first()).toBeVisible({ timeout: 25_000 });

  // Open the Code panel — the Mermaid source editor.
  await page.getByRole('button', { name: /^Code$/i }).click();
  const editor = page.locator('.code-pane .cm-content');
  await expect(editor).toBeVisible({ timeout: 10_000 });

  // The user edits the Mermaid — appends a node. It saves to the SAME mermaid file
  // (PUT with format:mermaid) and re-renders the preview.
  const saved = page.waitForResponse(
    (r) => r.request().method() === 'PUT' && /\/canvas\/scenes\//.test(r.url()),
    { timeout: 20_000 },
  );
  await editor.click();
  await page.keyboard.press('End');
  await page.keyboard.type('\n  C --> Z["User added"]');
  const resp = await saved;
  expect(resp.ok()).toBeTruthy();
  // The PUT body is a mermaid doc (NOT excalidraw) — same file the agent edits.
  const body = resp.request().postData() ?? '';
  expect(body).toContain('"format":"mermaid"');
  expect(body).toContain('User added');
});

test('live shell: the agent shell attaches at turn start (session id surfaced)', async ({ page }) => {
  test.setTimeout(90_000);
  const { ctx, base } = await apiCtx();
  await seedScene(
    ctx,
    base,
    'Shell Scene',
    'excalidraw',
    JSON.stringify({ type: 'excalidraw', elements: [] }),
  );
  await ctx.dispose();

  await openScene(page, 'Shell Scene');
  await expect(page.locator('.excali .excalidraw').first()).toBeVisible({ timeout: 30_000 });
  await askAi(page, 'a simple flow');
  // The canvas_session_started event sets the session id → the Assistant swaps the
  // empty hint for the live shell (Terminal) — the "Describe a diagram" hint goes away.
  await expect(page.getByText(/Describe a diagram and the agent draws it here/i)).toBeHidden({
    timeout: 20_000,
  });
});

test('switching Excalidraw → Mermaid does NOT corrupt the Mermaid scene', async ({ page }) => {
  test.setTimeout(90_000);
  const { ctx, base } = await apiCtx();
  const mmId = await seedScene(ctx, base, 'StayMermaid', 'mermaid', MERMAID.flowchart);
  await seedScene(ctx, base, 'AnExcali', 'excalidraw', EXCALI_SCENE);
  await ctx.dispose();

  // Open the Excalidraw scene and draw a shape (dirties it → it will autosave).
  await openScene(page, 'AnExcali');
  const cv = page.locator('.excalidraw canvas').first();
  await expect(cv).toBeVisible({ timeout: 30_000 });
  const box = await cv.boundingBox();
  const b = box!;
  await page.mouse.click(b.x + b.width * 0.5, b.y + b.height * 0.5);
  await page.keyboard.press('r');
  await page.mouse.move(b.x + b.width * 0.4, b.y + b.height * 0.4);
  await page.mouse.down();
  await page.mouse.move(b.x + b.width * 0.4 + 150, b.y + b.height * 0.4 + 110, { steps: 12 });
  await page.mouse.up();

  // Switch to the Mermaid scene — the OLD Excalidraw editor's onDestroy save must
  // target ITS OWN scene, never this Mermaid one.
  await page.locator('.scene-list .row', { hasText: 'StayMermaid' }).first().click();
  await expect(page.locator('.board svg').first()).toBeVisible({ timeout: 25_000 });
  await expect(page.getByText(/holds Excalidraw content/i)).toBeHidden();
  await expect(page.getByText(/Diagram error/i)).toBeHidden();

  // …and the Mermaid scene's stored doc is STILL Mermaid (not clobbered).
  const { ctx: ctx2, base: base2 } = await apiCtx();
  const scene = (await (await ctx2.get(`${base2}/api/v1/canvas/scenes/${mmId}`)).json()) as {
    doc_json: string;
  };
  await ctx2.dispose();
  expect(scene.doc_json).toContain('"format":"mermaid"');
  expect(scene.doc_json).not.toContain('excalidraw');
});

test.describe('canvas on a phone', () => {
  test.use({ viewport: { width: 390, height: 844 } });

  test('an Excalidraw board mounts on a phone', async ({ page }) => {
    test.setTimeout(60_000);
    const { ctx, base } = await apiCtx();
    await seedScene(ctx, base, 'Phone Ex', 'excalidraw', EXCALI_SCENE);
    await ctx.dispose();
    await openScene(page, 'Phone Ex');
    await expect(page.locator('.excali .excalidraw').first()).toBeVisible({ timeout: 30_000 });
  });
});
