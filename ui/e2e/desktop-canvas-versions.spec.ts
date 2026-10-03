import { test, expect } from '@playwright/test';
import { openPage } from './helpers';
import { apiCtx, seedWorkspace } from './seed';

// C1 + C5: a scene save carrying more than axum's 2 MB default (an Excalidraw
// board with a pasted screenshot) is accepted, every save keeps the pre-save
// doc in the scene's version history (throttled to one per 10 min), and the
// Assistant panel's "Restore previous version…" puts an older doc back.
// Needs a daemon built from this branch (OTTO_E2E_BIN=target/debug/ottod).

test.use({ serviceWorkers: 'block' });

const doc = (source: string) => ({ type: 'otto-canvas', version: 1, format: 'mermaid', source });

test('Canvas accepts large saves and restores a previous version', async ({ page }) => {
  const { ctx, base } = await apiCtx();
  const workspace = await seedWorkspace(ctx, base);
  const created = await ctx.post(`${base}/api/v1/workspaces/${workspace}/canvas/scenes`, {
    data: { title: 'Versioned canvas', doc: doc('flowchart LR\n A[Original] --> B[Board]') },
  });
  expect(created.ok()).toBeTruthy();
  const scene = await created.json();

  // ~3 MB body: over axum's 2 MB default, under the 25 MiB scene cap.
  const big = `flowchart LR\n A[Big] --> B[Board]\n%% ${'x'.repeat(3 * 1024 * 1024)}`;
  const put = await ctx.put(`${base}/api/v1/canvas/scenes/${scene.id}?summary=true`, { data: { doc: doc(big) } });
  expect(put.status(), 'a >2 MB scene save must not 413').toBe(200);

  // The original doc was snapshotted before that save.
  const versions = await (await ctx.get(`${base}/api/v1/canvas/scenes/${scene.id}/versions`)).json();
  expect(versions).toHaveLength(1);
  expect(versions[0].origin).toBe('user');
  // Put a small doc back so the editor stays light; inside the 10 min window
  // this save is NOT snapshotted again.
  await ctx.put(`${base}/api/v1/canvas/scenes/${scene.id}?summary=true`, {
    data: { doc: doc('flowchart LR\n A[Edited] --> B[Board]') },
  });
  expect(await (await ctx.get(`${base}/api/v1/canvas/scenes/${scene.id}/versions`)).json()).toHaveLength(1);
  // A version id never restores onto another scene.
  const stray = await ctx.post(`${base}/api/v1/canvas/scenes/not-a-scene/versions/${versions[0].id}/restore`);
  expect(stray.status()).toBe(404);
  await ctx.dispose();

  await page.addInitScript(id => localStorage.setItem('otto_workspace', id), workspace);
  await openPage(page, 'canvas');
  await page.locator('.scene-list .row', { hasText: 'Versioned canvas' }).getByRole('button').first().click();
  await page.getByRole('button', { name: /Ask AI/ }).click();
  await page.getByRole('button', { name: 'Restore previous version' }).click();
  await page.getByRole('button', { name: /Manual edit · / }).click();
  await expect(page.getByText('Canvas restored')).toBeVisible();
  await expect
    .poll(async () =>
      page.evaluate(async () => {
        const path = '/src/lib/stores/canvas.svelte.ts';
        const { canvas } = await import(path);
        return canvas.source as string | null;
      }),
    )
    .toContain('A[Original]');
});
