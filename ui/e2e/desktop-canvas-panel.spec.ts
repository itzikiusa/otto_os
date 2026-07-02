import { test, expect, type APIRequestContext } from '@playwright/test';
import { apiCtx, seedWorkspace, seedShellSession } from './seed';

// ── E2E: the agent session's right-panel "Canvas" tab ──────────────────────────
//
// Every `agent`-kind session gets a right-panel "Canvas" tab (CanvasPanel.svelte,
// GET/POST/DELETE /sessions/{id}/canvas-refs) listing scenes referenced by that
// session — an expandable inline SVG preview, "Open in Canvas", "Detach", and an
// "Attach scene…" picker. Desktop-only (the ≥1025px 3-pane shell mounts
// `RightPanel`; mobile/tablet host it in a drawer instead) — self-skips on the
// mobile/tablet projects like `desktop-shell.spec.ts:18`.

let workspaceId = '';
let sessionId = '';
let sceneId = '';
const SCENE_TITLE = 'Panel Mermaid';
const MERMAID_SRC = 'flowchart TD\n  A(["Start"]) --> B["Panel Preview"]';

async function seedMermaidScene(
  ctx: APIRequestContext,
  base: string,
  wsId: string,
  title: string,
  source: string,
): Promise<string> {
  const doc = { type: 'otto-canvas', version: 1, format: 'mermaid', source };
  const r = await ctx.post(`${base}/api/v1/workspaces/${wsId}/canvas/scenes`, {
    data: { title, doc },
  });
  expect(r.ok()).toBeTruthy();
  return ((await r.json()) as { id: string }).id;
}

test.beforeEach(async ({ page }, info) => {
  test.skip(info.project.name !== 'desktop-browser', 'desktop-browser project only');
  if (!workspaceId) {
    const { ctx, base } = await apiCtx();
    workspaceId = await seedWorkspace(ctx, base);
    sessionId = await seedShellSession(ctx, base, workspaceId); // kind:'agent' → the right panel shows
    sceneId = await seedMermaidScene(ctx, base, workspaceId, SCENE_TITLE, MERMAID_SRC);
    await ctx.dispose();
  }
  await page.addInitScript((w) => {
    localStorage.setItem('otto_workspace', w as string);
    localStorage.setItem('otto_rail_expanded', '0');
  }, workspaceId);
});

test('Canvas panel: attach a scene, preview it, open in Canvas, detach', async ({ page }) => {
  test.setTimeout(90_000);
  await page.goto(`/#/agents/${sessionId}`);
  await expect(page.locator('.term-host').first()).toBeVisible({ timeout: 30_000 });

  // Collapsed by default — the icon strip's "Canvas" button opens the panel
  // straight onto that tab (RightPanel.svelte's openRight()).
  await page.locator('.rstrip').getByRole('button', { name: 'Canvas' }).click();
  const panel = page.locator('.canvas-panel');
  await expect(panel).toBeVisible({ timeout: 10_000 });
  await expect(page.getByText('No canvases referenced')).toBeVisible({ timeout: 10_000 });

  // Attach the seeded scene via the picker.
  await panel.getByRole('button', { name: /Attach scene/i }).click();
  await panel.locator('.candidates').getByRole('button', { name: SCENE_TITLE }).click();
  const row = panel.locator('.ref-row', { hasText: SCENE_TITLE });
  await expect(row).toBeVisible({ timeout: 15_000 });
  await expect(row.locator('.chip.fmt-mermaid')).toHaveText('mermaid');

  // The reference is real on the server too (not just optimistic UI state).
  const { ctx, base } = await apiCtx();
  const refsResp = await ctx.get(`${base}/api/v1/sessions/${sessionId}/canvas-refs`);
  expect(refsResp.ok()).toBeTruthy();
  const refs = (await refsResp.json()) as { id: string }[];
  expect(refs.some((r) => r.id === sceneId)).toBeTruthy();
  await ctx.dispose();

  // Expand — the inline SVG preview renders from the scene's live source.
  await row.locator('.ref-title').click();
  await expect(row.locator('.ref-preview svg').first()).toBeVisible({ timeout: 20_000 });

  // "Open in Canvas" navigates to the Canvas module with the scene open.
  await row.getByRole('button', { name: 'Open in Canvas' }).click();
  await expect(page).toHaveURL(/#\/canvas/);
  await expect(page.locator('.canvas-page')).toBeVisible({ timeout: 20_000 });
  await expect(page.locator('.scene-list .row.active', { hasText: SCENE_TITLE })).toBeVisible({
    timeout: 15_000,
  });
  await expect(page.locator('.board svg').first()).toBeVisible({ timeout: 20_000 });

  // Back to the session — the panel (and its ref) is still there.
  await page.goBack();
  await expect(page.locator('.term-host').first()).toBeVisible({ timeout: 20_000 });
  const rowAfterBack = page.locator('.canvas-panel .ref-row', { hasText: SCENE_TITLE });
  await expect(rowAfterBack).toBeVisible({ timeout: 15_000 });

  // Detach — the row disappears and the empty state returns.
  await rowAfterBack.getByRole('button', { name: 'Detach from this session' }).click();
  await expect(rowAfterBack).toHaveCount(0);
  await expect(page.getByText('No canvases referenced')).toBeVisible({ timeout: 10_000 });
});

test('Canvas panel: a live API attach (canvas_refs_changed) updates the panel without reload', async ({
  page,
}) => {
  test.setTimeout(60_000);
  await page.goto(`/#/agents/${sessionId}`);
  await expect(page.locator('.term-host').first()).toBeVisible({ timeout: 30_000 });

  await page.locator('.rstrip').getByRole('button', { name: 'Canvas' }).click();
  const panel = page.locator('.canvas-panel');
  await expect(panel).toBeVisible({ timeout: 10_000 });
  await expect(page.getByText('No canvases referenced')).toBeVisible({ timeout: 10_000 });

  // Attach from OUTSIDE the page — a raw API call (an MCP tool or another
  // client would make this exact call), never touching this tab.
  const { ctx, base } = await apiCtx();
  const r = await ctx.post(`${base}/api/v1/sessions/${sessionId}/canvas-refs`, {
    data: { scene_id: sceneId },
  });
  expect(r.ok()).toBeTruthy();
  await ctx.dispose();

  // The daemon broadcasts `canvas_refs_changed` over /ws/events; the panel's
  // canvasRefsBus subscriber refetches on its own — no reload, no user action.
  await expect(panel.locator('.ref-row', { hasText: SCENE_TITLE })).toBeVisible({
    timeout: 15_000,
  });
});
