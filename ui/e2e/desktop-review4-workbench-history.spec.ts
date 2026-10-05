import { test, expect, type APIRequestContext } from '@playwright/test';
import { apiCtx, seedWorkspace } from './seed';

// Route fixture bounds history deterministically; the Rust route regression
// separately seeds 50k actual SQLite revisions. No user file/history is touched.
let ctx: APIRequestContext, base: string, workspaceId: string;
test.beforeEach(async ({ page }, info) => {
  test.skip(info.project.name !== 'desktop-browser', 'desktop browser acceptance');
  ({ ctx, base } = await apiCtx()); workspaceId = await seedWorkspace(ctx, base);
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.addInitScript(id => { localStorage.setItem('otto_workspace', id); localStorage.setItem('otto_firstrun_dismissed', '1'); }, workspaceId);
});
test.afterEach(async () => { await ctx?.dispose(); });

test('history pages stay bounded while oldest revision remains selectable, comparable and restorable', async ({ page }) => {
  const created = await ctx.post(`${base}/api/v1/workspaces/${workspaceId}/workbench/docs`, {
    data: { name: 'bounded-history.txt', language: 'txt', content: 'current content' },
  });
  expect(created.ok()).toBe(true);
  const doc = await created.json();
  let head = 305;
  const pages: { before: number | null; limit: number | null }[] = [];
  const details: number[] = [], restores: number[] = [];
  const revision = (seq: number) => ({ seq, kind: 'checkpoint', content_hash: 'a'.repeat(64), size: 10, created_at: '2026-10-05T00:00:00Z', updated_at: '2026-10-05T00:00:00Z', saves: 1, restored_from: null });
  await page.route(`**/workbench/docs/${doc.id}/revisions*`, async route => {
    const u = new URL(route.request().url());
    const limit = u.searchParams.has('limit') ? Number(u.searchParams.get('limit')) : null;
    const before = u.searchParams.has('before_seq') ? Number(u.searchParams.get('before_seq')) : null;
    pages.push({ limit, before });
    // Model the old unbounded endpoint when a client does not request a page,
    // so the test independently requires the UI to request bounded metadata.
    const end = Math.min(head, (before ?? head + 1) - 1), count = Math.min(end, limit ?? head);
    await route.fulfill({ json: Array.from({ length: count }, (_, i) => revision(end - i)) });
  });
  await page.route(`**/workbench/docs/${doc.id}/revisions/**`, async route => {
    const match = new URL(route.request().url()).pathname.match(/\/revisions\/(\d+)(\/restore)?$/);
    if (!match) return route.fallback();
    const seq = Number(match[1]);
    if (match[2]) { restores.push(seq); head++; await route.fulfill({ json: { ...doc, rev: head, content: `content of revision ${seq}` } }); }
    else { details.push(seq); await route.fulfill({ json: { ...revision(seq), doc_id: doc.id, content: `content of revision ${seq}` } }); }
  });
  await page.goto('/#/workbench');
  await expect(page.locator('.shell')).toBeVisible({ timeout: 30_000 });
  await page.getByTestId('wb-file-row').filter({ hasText: doc.name }).click();
  await page.getByTestId('wb-history-toggle').click();
  const history = page.getByTestId('wb-history'), rows = history.getByTestId('wb-rev-row');
  await expect(rows).toHaveCount(100);
  for (const oldestVisible of [106, 6, 1]) {
    await history.getByRole('button', { name: 'Older revisions', exact: true }).click();
    await expect(rows.last()).toHaveAttribute('data-seq', String(oldestVisible));
    expect(await rows.count()).toBeLessThanOrEqual(100);
  }
  await rows.filter({ hasText: '#1' }).last().click();
  await expect.poll(() => details.includes(1)).toBe(true);
  const compare = history.getByLabel('Compare revision', { exact: true });
  await compare.fill('305'); await compare.press('Enter');
  await expect.poll(() => details.includes(305)).toBe(true);
  expect(await history.locator('option').count()).toBeLessThanOrEqual(102);
  await history.getByTestId('wb-rev-restore').click();
  await page.locator('.sheet[role="dialog"]').last().getByRole('button', { name: 'Restore', exact: true }).click();
  await expect.poll(() => restores).toEqual([1]);
  await expect(rows.first()).toHaveAttribute('data-seq', '306');
  expect(await rows.count()).toBeLessThanOrEqual(100);
  expect(pages.every(p => p.limit !== null && p.limit >= 1 && p.limit <= 200)).toBe(true);
  expect(pages.some(p => p.before === 206)).toBe(true);
  expect(pages.some(p => p.before === 106)).toBe(true);
  expect(pages.some(p => p.before === 6)).toBe(true);
});
