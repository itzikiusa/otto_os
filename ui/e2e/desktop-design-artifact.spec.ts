import { test, expect, type APIRequestContext, type Page } from '@playwright/test';
import { apiCtx, seedWorkspace } from './seed';
import { expectFullyInViewport, expectNoHorizontalOverflow } from './helpers';

// ─────────────────────────────────────────────────────────────────────────────
// Design Hall — the artifact view (desktop-browser), against the REAL
// /design/* routes (OTTO_E2E_BIN=<repo>/target/debug/ottod):
//
//   • a mirror of a Canvas board shows ONE banner; "Edit here" adopts it
//     (`POST …/adopt`) and editing works in place, autosaved;
//   • edits autosave (no Save button): the header says Saving… → Saved;
//   • the Versions tab lists named versions by message, folds autosaves,
//     views one version read-only with "Back to current", moves with ↑/↓,
//     and approves a version from its row — the pill then reads
//     "Approved v1 · edited";
//   • Shipped without an approved version is a designed message (server 409);
//   • an archived design offers Unarchive in ⋯;
//   • Compare's Changes view shows a failed load inline with Retry;
//   • ⋯ → Export lists PNG / SVG / source for a Mermaid diagram;
//   • at 390 px the Details button opens Otto · Versions · Links.
// ─────────────────────────────────────────────────────────────────────────────

const V1 = '/api/v1';
let wsId = '';
const stamp = `${Date.now().toString(36)}${process.pid.toString(36)}`;

async function postJson(ctx: APIRequestContext, url: string, data: unknown): Promise<any> {
  const r = await ctx.post(url, { data });
  expect(r.ok(), `${url} → ${r.status()} ${await r.text()}`).toBeTruthy();
  return r.json();
}

/** An HTML design with v1 (created), v2 (a named version) and v3 (an autosave). */
async function threeVersions(title: string): Promise<{ id: string; v1: string }> {
  const { ctx, base } = await apiCtx();
  const a = await postJson(ctx, `${base}${V1}/design/artifacts`, {
    workspace_id: wsId, format: 'html', studio: 'frames', title, content: '<h1>One</h1>',
  });
  const v2 = await postJson(ctx, `${base}${V1}/design/artifacts/${a.artifact.id}/versions`, {
    content: '<h1>Two</h1>', base_version: a.version.id, message: 'Bolder headline',
  });
  const put = await ctx.put(`${base}${V1}/design/artifacts/${a.artifact.id}/content`, {
    data: { content: '<h1>Three</h1>', base_version: v2.version.id },
  });
  expect(put.ok(), await put.text()).toBeTruthy();
  await ctx.dispose();
  return { id: a.artifact.id, v1: a.version.id };
}

test.beforeAll(async () => {
  const { ctx, base } = await apiCtx();
  wsId = await seedWorkspace(ctx, base);
  await ctx.dispose();
});

test.beforeEach(async ({ page }) => {
  await page.addInitScript((id) => localStorage.setItem('otto_workspace', id as string), wsId);
});

async function openDesign(page: Page, id: string): Promise<void> {
  await page.goto(`/#/design/a/${id}`);
  await expect(page.getByTestId('design-more')).toBeVisible({ timeout: 30_000 });
}

test('a Canvas mirror shows one banner; Edit here adopts it and editing autosaves in place', async ({ page }) => {
  const { ctx, base } = await apiCtx();
  const scene = await postJson(ctx, `${base}${V1}/workspaces/${wsId}/canvas/scenes`, {
    title: `Mirrored flow ${stamp}`,
    doc: { type: 'otto-canvas', version: 1, format: 'mermaid', source: 'flowchart LR\n  A --> B\n' },
  });
  // The sync mirrors the board into the Hall.
  let mirrorId = '';
  await expect
    .poll(async () => {
      const r = await ctx.get(`${base}${V1}/design/artifacts?source_kind=canvas_scene&source_id=${scene.id}`);
      const rows = r.ok() ? ((await r.json()) as { id: string }[]) : [];
      mirrorId = rows[0]?.id ?? '';
      return mirrorId;
    }, { timeout: 30_000 })
    .not.toBe('');
  await ctx.dispose();

  await openDesign(page, mirrorId);
  const banner = page.getByTestId('design-mirror-banner');
  await expect(banner).toHaveCount(1);
  await expect(banner).toContainText('From Canvas');
  await expect(banner).toContainText('This design mirrors a Canvas board.');
  // Read-only until adopted: no save state, Otto explains.
  await expect(page.getByTestId('design-save-state')).toHaveCount(0);

  const adopt = page.waitForRequest((r) => r.method() === 'POST' && r.url().endsWith(`/design/artifacts/${mirrorId}/adopt`));
  await page.getByTestId('design-adopt').click();
  await adopt;
  await expect(banner).toHaveCount(0);
  await expect(page.getByTestId('design-save-state')).toHaveAttribute('data-state', 'saved');
  await expectNoHorizontalOverflow(page);
});

test('edits autosave; the Versions tab names, folds, views and approves versions', async ({ page }) => {
  const { id } = await threeVersions(`Versions ${stamp}`);
  await openDesign(page, id);
  // No Save button: a quiet indicator.
  await expect(page.getByTestId('design-save-state')).toHaveAttribute('data-state', 'saved');

  await page.getByTestId('design-tab-versions').click();
  const rows = page.getByTestId('design-version-row');
  await expect(rows).toHaveCount(3);
  await expect(rows.filter({ hasText: 'Bolder headline' })).toHaveAttribute('data-kind', 'named');

  // View v1 alone (read-only), then go back to the current version.
  await rows.filter({ hasText: 'v1' }).locator('button.row-main').click();
  await expect(page.getByTestId('design-viewing')).toContainText('Viewing v1');
  await page.getByTestId('design-back-to-current').click();
  await expect(page.getByTestId('design-viewing')).toHaveCount(0);

  // ↑/↓ move through the rows.
  const mains = rows.locator('button.row-main');
  await mains.first().focus();
  await page.keyboard.press('ArrowDown');
  await expect(mains.nth(1)).toBeFocused();

  // Approve v1 from its row menu; the design keeps newer versions, so the pill says so.
  await rows.filter({ hasText: 'v1' }).getByTestId('design-version-menu').click();
  await expectFullyInViewport(page, page.locator('.ctx-menu'), 'version row menu');
  await page.locator('.ctx-menu').getByRole('menuitem', { name: 'Approve this version…' }).click();
  await page.getByRole('dialog', { name: 'Approve design' }).getByRole('button', { name: 'Approve v1' }).click();
  await expect(page.getByTestId('design-status')).toContainText('Approved v1 · edited');
  await expect(rows.filter({ hasText: 'v1' })).toContainText('Approved');

  // An edit autosaves as a new version.
  await page.getByTestId('design-source-toggle').click();
  await page.locator('.cm-content').first().fill('<h1>Four</h1>');
  await expect(page.getByTestId('design-dirty')).toBeVisible();
  await expect(page.getByTitle('Current version: v4')).toBeVisible();
  await expect(page.getByTestId('design-save-state')).toHaveAttribute('data-state', 'saved');
});

test('Shipped without an approved version is explained; an archived design can be unarchived', async ({ page }) => {
  const { ctx, base } = await apiCtx();
  const a = await postJson(ctx, `${base}${V1}/design/artifacts`, {
    workspace_id: wsId, format: 'mermaid', studio: 'whiteboard', title: `Status ${stamp}`, content: 'flowchart LR\n  A --> B\n',
  });
  await ctx.dispose();
  await openDesign(page, a.artifact.id);
  await page.getByTestId('design-status').click();
  await page.locator('.ctx-menu').getByRole('menuitem', { name: 'Shipped' }).click();
  await expect(page.getByText('Approve a version before shipping')).toBeVisible();

  // ⋯ → Export by format (Mermaid: PNG, SVG and the source).
  await page.getByTestId('design-more').click();
  await page.locator('.ctx-menu').getByRole('menuitem', { name: 'Export…' }).click();
  const exportMenu = page.locator('.ctx-menu');
  await expect(exportMenu.getByRole('menuitem', { name: 'PNG image' })).toBeVisible();
  await expect(exportMenu.getByRole('menuitem', { name: 'SVG image' })).toBeVisible();
  await expect(exportMenu.getByRole('menuitem', { name: 'Source (.mmd)' })).toBeVisible();
  await page.keyboard.press('Escape');

  const { ctx: c2, base: b2 } = await apiCtx();
  const del = await c2.delete(`${b2}${V1}/design/artifacts/${a.artifact.id}`);
  expect(del.ok()).toBeTruthy();
  await c2.dispose();
  await openDesign(page, a.artifact.id);
  await page.getByTestId('design-more').click();
  await page.locator('.ctx-menu').getByRole('menuitem', { name: 'Unarchive' }).click();
  await expect(page.getByText('Moved back to Draft')).toBeVisible();
  await expect(page.getByTestId('design-status')).toContainText('Draft');
});

test('Compare shows a version that failed to load inline, with Retry', async ({ page }) => {
  const { id, v1 } = await threeVersions(`Compare ${stamp}`);
  let fail = true;
  await page.route(`**/design/artifacts/${id}/versions/${v1}/content`, (route) =>
    fail ? route.fulfill({ status: 500, contentType: 'application/json', body: '{"code":"internal","message":"disk unavailable"}' }) : route.continue(),
  );
  await openDesign(page, id);
  await page.getByTestId('design-tab-versions').click();
  await page.getByTestId('design-version-row').filter({ hasText: 'v1' }).locator('button.row-main').click();
  await page.getByTestId('design-version-compare').click();
  await page.getByRole('tab', { name: 'Changes' }).click();
  const err = page.getByTestId('design-compare-error');
  await expect(err).toContainText('disk unavailable');
  fail = false;
  await err.getByRole('button', { name: 'Retry' }).click();
  await expect(err).toHaveCount(0);
  await expect(page.getByTestId('design-compare-modal')).toContainText('One');
});

test('at phone width the Details button opens Otto · Versions · Links', async ({ page }) => {
  const { id } = await threeVersions(`Phone ${stamp}`);
  await page.setViewportSize({ width: 390, height: 844 });
  await openDesign(page, id);
  await expectNoHorizontalOverflow(page);
  await page.getByRole('button', { name: 'Show design details', exact: true }).click();
  for (const tab of ['otto', 'versions', 'links'] as const) {
    await page.getByTestId(`design-tab-${tab}`).click();
    await expect(page.getByTestId(`design-tab-${tab}`)).toHaveAttribute('aria-selected', 'true');
  }
  await expect(page.getByTestId('design-find-references')).toBeVisible();
  await expectNoHorizontalOverflow(page);
  await page.getByRole('button', { name: 'Back to design', exact: true }).click();
  await expect(page.getByTestId('design-stage')).toBeVisible();
});
