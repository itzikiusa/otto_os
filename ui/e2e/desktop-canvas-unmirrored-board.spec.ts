import { test, expect, type Page } from '@playwright/test';
import { openPage } from './helpers';
import { apiCtx, seedWorkspace } from './seed';

// `#/canvas` forwards a board to its Design Hall copy — but a board that has
// NO copy (the daemon could not make one: too large with its images, an image
// missing) must still open. Since Canvas only forwards, it opens in the kept
// Canvas editor (CanvasPage, lazy-loaded by CanvasRedirect) under one line
// that says why — never a toast and a dead end.
//
// The board below references an image whose bytes are gone, so its portable
// copy cannot be built and the daemon never mirrors it (true of the round-2
// daemon too: this spec exercises the UI path alone). A plain board is the
// control: it still forwards into the Hall.

test.use({ serviceWorkers: 'block' });

const doc = (source: string) => ({ type: 'otto-canvas', version: 1, format: 'mermaid', source });
const MISSING_IMAGE = `otto-canvas-file:${'a'.repeat(64)}`;

async function seedScene(title: string, source: string): Promise<{ workspace: string; id: string }> {
  const { ctx, base } = await apiCtx();
  const workspace = await seedWorkspace(ctx, base);
  const created = await ctx.post(`${base}/api/v1/workspaces/${workspace}/canvas/scenes`, {
    data: { title, doc: doc(source) },
  });
  expect(created.ok(), await created.text()).toBeTruthy();
  const scene = await created.json();
  await ctx.dispose();
  return { workspace, id: scene.id as string };
}

async function openedSceneId(page: Page): Promise<string | null> {
  return page.evaluate(async () => {
    const path = '/src/lib/stores/canvas.svelte.ts';
    const { canvas } = await import(path);
    return canvas.currentId as string | null;
  });
}

test('a scene with no Hall copy opens in the old editor, saying why', async ({ page }) => {
  const { workspace, id } = await seedScene('Unmirrored board', `flowchart LR\n A[Kept] --> B[Board]\n%% ${MISSING_IMAGE}`);
  await page.addInitScript((ws) => localStorage.setItem('otto_workspace', ws), workspace);
  await openPage(page, `canvas/${id}`);

  const note = page.getByTestId('canvas-legacy-note');
  await expect(note).toBeVisible({ timeout: 15_000 });
  await expect(note).toContainText('isn’t in Design Hall yet');
  await expect(note).toContainText('Canvas editor');
  // The board itself is open in the editor, and the route stayed on Canvas.
  await expect(page.locator('.canvas-shell')).toContainText('Unmirrored board');
  await expect.poll(() => openedSceneId(page), { timeout: 10_000 }).toBe(id);
  expect(page.url()).toContain(`#/canvas/${id}`);
  await expect(page.getByText('It appears here within a few seconds')).toHaveCount(0);
});

test('#/canvas/<id>/editor opens the board in the editor at once', async ({ page }) => {
  const { workspace, id } = await seedScene('Direct board', `flowchart LR\n A[Direct] --> B\n%% ${MISSING_IMAGE}`);
  await page.addInitScript((ws) => localStorage.setItem('otto_workspace', ws), workspace);
  await openPage(page, `canvas/${id}/editor`);
  await expect(page.getByTestId('canvas-legacy-note')).toBeVisible({ timeout: 10_000 });
  await expect.poll(() => openedSceneId(page), { timeout: 10_000 }).toBe(id);
});

test('a board with a Hall copy still forwards into Design Hall', async ({ page }) => {
  const { workspace, id } = await seedScene('Mirrored board', 'flowchart LR\n A[Mirrored] --> B');
  // Wait for the live sync's copy first, so the redirect's short wait never races it.
  const { ctx, base } = await apiCtx();
  await expect
    .poll(async () => {
      const r = await ctx.get(`${base}/api/v1/design/artifacts?source_kind=canvas_scene&source_id=${id}&limit=1`);
      return r.ok() ? ((await r.json()) as unknown[]).length : -1;
    }, { timeout: 15_000 })
    .toBe(1);
  await ctx.dispose();
  await page.addInitScript((ws) => localStorage.setItem('otto_workspace', ws), workspace);
  await openPage(page, `canvas/${id}`);
  await expect.poll(() => page.url(), { timeout: 15_000 }).toMatch(/#\/design\/a\//);
  await expect(page.getByTestId('canvas-legacy-note')).toHaveCount(0);
});
