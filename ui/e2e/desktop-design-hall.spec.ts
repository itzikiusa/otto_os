import { test, expect, type APIRequestContext, type Page } from '@playwright/test';
import { apiCtx, seedWorkspace } from './seed';
import { expectFullyInViewport, expectNoHorizontalOverflow } from './helpers';

// ─────────────────────────────────────────────────────────────────────────────
// Design Hall (desktop-browser) — against the REAL /design/* routes, so it needs
// a daemon built from this tree (OTTO_E2E_BIN=<repo>/target/debug/ottod).
//
//   • the lobby shows all seven studios, and the header "New ▾" menu, the status
//     menu and a data-driven "Move to project…" menu (40 projects, enough to
//     overflow the window) stay fully inside the viewport;
//   • create a draft from the lobby prompt → it opens → edit the source → Save
//     makes v2 → Compare v1 with the current version → Restore v1 asks first and
//     saves it as v3 (history keeps growing — nothing is rewound);
//   • an explicit link made in the Links panel shows as "Used in" on the target;
//   • the References panel searches the team library.
// ─────────────────────────────────────────────────────────────────────────────

const V1 = '/api/v1';
let wsId = '';
let targetId = '';
const stamp = Date.now().toString(36);
const TARGET_TITLE = `Tier card component ${stamp}`;

async function postJson(ctx: APIRequestContext, url: string, data: unknown): Promise<any> {
  const r = await ctx.post(url, { data });
  expect(r.ok(), `${url} → ${r.status()} ${await r.text()}`).toBeTruthy();
  return r.json();
}

test.beforeAll(async () => {
  const { ctx, base } = await apiCtx();
  wsId = await seedWorkspace(ctx, base);
  // Enough projects that the "Move to project…" menu is taller than the window.
  for (let i = 0; i < 40; i++) {
    await postJson(ctx, `${base}${V1}/design/projects`, { workspace_id: wsId, name: `E2E project ${String(i).padStart(2, '0')} ${stamp}` });
  }
  // The link target (an HTML frame) the Links + References flows look for.
  const res = await postJson(ctx, `${base}${V1}/design/artifacts`, {
    workspace_id: wsId,
    format: 'html',
    studio: 'frames',
    title: TARGET_TITLE,
    content: '<!doctype html><html><body><h1>Tier card</h1></body></html>',
  });
  targetId = res.artifact.id;
  await ctx.dispose();
});

test.beforeEach(async ({ page }) => {
  await page.addInitScript((id) => {
    localStorage.setItem('otto_workspace', id as string);
  }, wsId);
});

async function openDesign(page: Page, route = 'design'): Promise<void> {
  await page.goto(`/#/${route}`);
  await expect(page.locator('.shell')).toBeVisible({ timeout: 30_000 });
}

test('the lobby shows every studio and its menus stay inside the viewport', async ({ page }) => {
  await openDesign(page);
  await expect(page.getByTestId('design-lobby')).toBeVisible();
  const studios = page.getByTestId('design-studios').locator('button');
  await expect(studios).toHaveCount(7);
  for (const name of ['Frames', 'Graphics', 'Site Studio', '3D Studio', 'Whiteboard', 'Brand Kit', 'Spatial Hall']) {
    await expect(studios.filter({ hasText: name })).toHaveCount(1);
  }
  await expectNoHorizontalOverflow(page);

  // "New ▾" — every studio row, planned ones visible but disabled.
  await page.getByTestId('design-new').click();
  const menu = page.locator('.ctx-menu');
  await page.waitForTimeout(100); // post-render clamp
  await expectFullyInViewport(page, menu, 'New menu');
  await expect(menu.getByRole('menuitem', { name: 'Frame screen (HTML)' })).toBeVisible();
  await page.keyboard.press('Escape');

  // A planned studio opens an explanation, never a dead click.
  await page.getByTestId('design-studio-spatial').click();
  await expect(page.getByTestId('design-studio-note')).toContainText('v3');
});

test('create → edit → new version → compare → restore', async ({ page }) => {
  await openDesign(page);
  const brief = `Checkout confirmation screen ${stamp}`;
  await page.getByTestId('design-prompt').fill(brief);
  await page.getByTestId('design-prompt-create').click();

  // The draft opens in the artifact view at v1.
  await expect(page).toHaveURL(/#\/design\/a\//);
  await expect(page.getByTestId('design-stage')).toBeVisible();
  const chips = page.getByTestId('design-version-chip');
  await expect(chips).toHaveCount(1);

  // Edit the HTML source, then Save → v2.
  await page.getByTestId('design-source-toggle').click();
  const editor = page.locator('.cm-content').first();
  await editor.click();
  await page.keyboard.press('ControlOrMeta+End');
  await page.keyboard.type('\n<!-- e2e edit -->');
  await expect(page.getByTestId('design-dirty')).toBeVisible();
  await page.getByTestId('design-save').click();
  await expect(chips).toHaveCount(2);
  await expect(page.getByTestId('design-dirty')).toHaveCount(0);

  // Compare v1 with the current version, then Restore v1 (asks first).
  await chips.filter({ hasText: 'v1' }).first().click();
  await page.getByTestId('design-compare').click();
  const cmp = page.getByTestId('design-compare-modal');
  await expect(cmp).toBeVisible();
  await page.getByRole('tab', { name: 'Changes' }).click();
  await expect(cmp).toContainText('e2e edit');
  await page.getByTestId('design-restore').click();
  const confirm = page.getByRole('dialog', { name: 'Restore version' });
  await expect(confirm).toContainText('nothing is lost');
  await confirm.getByRole('button', { name: 'Restore v1' }).click();
  await expect(chips).toHaveCount(3);
  await expect(chips.filter({ hasText: 'v3' })).toContainText('current');

  // The status menu stays inside the viewport too.
  await page.getByTestId('design-status').click();
  await page.waitForTimeout(100);
  await expectFullyInViewport(page, page.locator('.ctx-menu'), 'status menu');
  await page.keyboard.press('Escape');

  // "Move to project…" lists all 40+ projects: clamped + scrollable, never off-screen.
  await page.getByTestId('design-more').click();
  await page.locator('.ctx-menu').getByRole('menuitem', { name: 'Move to project…' }).click();
  const moveMenu = page.locator('.ctx-menu');
  await expect(moveMenu).toBeVisible();
  await page.waitForTimeout(100);
  await expectFullyInViewport(page, moveMenu, 'move-to-project menu');
  await page.keyboard.press('Escape');
});

test('a link added here shows up as "Used in" on the other design', async ({ page }) => {
  const { ctx, base } = await apiCtx();
  const src = await postJson(ctx, `${base}${V1}/design/artifacts`, {
    workspace_id: wsId,
    format: 'mermaid',
    title: `Checkout flow ${stamp}`,
    content: 'flowchart LR\n  A[Cart] --> B[Pay]\n',
  });
  await ctx.dispose();

  await openDesign(page, `design/a/${src.artifact.id}`);
  await page.getByTestId('design-tab-links').click();
  await page.getByTestId('design-add-link').click();
  await page.getByTestId('design-link-search').fill(TARGET_TITLE);
  await page.getByRole('option', { name: new RegExp(TARGET_TITLE) }).click();
  await page.getByTestId('design-add-link-confirm').click();
  await expect(page.getByTestId('design-link-out').filter({ hasText: TARGET_TITLE })).toBeVisible();

  // The target sees it under Used in.
  await openDesign(page, `design/a/${targetId}`);
  await page.getByTestId('design-tab-links').click();
  await expect(page.getByTestId('design-link-in').filter({ hasText: `Checkout flow ${stamp}` })).toBeVisible();
});

test('references search finds designs in the team library', async ({ page }) => {
  const { ctx, base } = await apiCtx();
  const other = await postJson(ctx, `${base}${V1}/design/artifacts`, {
    workspace_id: wsId,
    format: 'd2',
    title: `Member journey ${stamp}`,
    content: 'join -> earn -> spend\n',
  });
  await ctx.dispose();

  await openDesign(page, `design/a/${other.artifact.id}`);
  await page.getByTestId('design-tab-references').click();
  await page.getByTestId('design-ref-search').fill('Tier card');
  await expect(page.getByTestId('design-ref-hit').filter({ hasText: TARGET_TITLE })).toBeVisible();
  // Adding it as a reference marks it as referenced (a pinned `references` link).
  await page.getByTestId('design-ref-hit').filter({ hasText: TARGET_TITLE }).getByTestId('design-ref-add').click();
  await expect(page.getByTestId('design-ref-hit').filter({ hasText: TARGET_TITLE })).toContainText('Referenced');
});
