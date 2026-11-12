import { test, expect, type APIRequestContext, type Page } from '@playwright/test';
import { apiCtx, seedWorkspace } from './seed';
import { expectFullyInViewport, expectNoHorizontalOverflow } from './helpers';

// ─────────────────────────────────────────────────────────────────────────────
// Design Hall (desktop-browser) — against the REAL /design/* routes, so it needs
// a daemon built from this tree (OTTO_E2E_BIN=<repo>/target/debug/ottod).
//
//   • the lobby is one prompt, All designs and a rail (no studio tiles, Spatial,
//     Continue or Linked-to-Product); the header "New ▾" and ⋯ menus, the status
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
// Per worker: two workers can load this file in the same millisecond, and the
// references search spans every workspace's designs.
const stamp = `${Date.now().toString(36)}${process.pid.toString(36)}`;
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

test('the lobby is one prompt, All designs and a rail; its menus stay inside the viewport', async ({ page }) => {
  await openDesign(page);
  await expect(page.getByTestId('design-lobby')).toBeVisible();
  // The retired regions are gone: no studio tiles, no Spatial tab, no
  // Continue strip, no Linked-to-Product tree.
  await expect(page.getByTestId('design-studios')).toHaveCount(0);
  await expect(page.getByTestId('design-spatial-tab')).toHaveCount(0);
  await expect(page.getByTestId('design-continue')).toHaveCount(0);
  await expect(page.getByText('Linked to Product')).toHaveCount(0);
  await expect(page.getByTestId('design-all')).toBeVisible();
  await expect(page.getByTestId('design-grid').getByTestId('design-artifact-card').first()).toBeVisible({ timeout: 15_000 });
  await expect(page.getByTestId('design-needs-review')).toBeVisible();
  // One primary action on the page: Generate.
  await expect(page.locator('.btn.primary:visible')).toHaveCount(1);
  await expect(page.getByTestId('design-prompt-generate')).toHaveClass(/primary/);
  await expectNoHorizontalOverflow(page);

  // "New ▾" — one row per studio, every one enabled; no roadmap rows.
  await page.getByTestId('design-new').click();
  const menu = page.locator('.ctx-menu');
  await page.waitForTimeout(100); // ui-guards: allow — post-render clamp before measuring
  await expectFullyInViewport(page, menu, 'New menu');
  for (const name of ['Frame screen', 'Graphic', 'Site', '3D scene', 'Whiteboard diagram or board', 'Brand kit', 'Import file…', 'New project…']) {
    await expect(menu.getByRole('menuitem', { name, exact: true })).toBeEnabled();
  }
  await expect(menu.getByText(/Exhibition|planned/)).toHaveCount(0);
  await page.keyboard.press('Escape');

  // Brand Kit and Team design rules live in the header ⋯.
  await page.getByTestId('design-more').click();
  await expectFullyInViewport(page, menu, 'More menu');
  await menu.getByRole('menuitem', { name: 'Team design rules' }).click();
  await expect(page).toHaveURL(/#\/design\/learned$/);
  await expect(page.getByTestId('design-learned-tab-pending')).toContainText('Proposed');

  // A studio page explains what the studio edits — no roadmap notes.
  await openDesign(page, 'design/studio/frames');
  await expect(page.getByTestId('design-studio-note')).toContainText('HTML screen');
  await expect(page.getByTestId('design-studio-note')).not.toContainText(/v2|planned/);

  // The retired Spatial route lands on the lobby.
  await openDesign(page, 'design/spatial');
  await expect(page.getByTestId('design-all')).toBeVisible();
});

test('create → edit → new version → compare → restore', async ({ page }) => {
  await openDesign(page);
  const brief = `Checkout confirmation screen ${stamp}`;
  await page.getByTestId('design-prompt').fill(brief);
  // "Draft only" sits in the prompt's ⋯ popover (Generate is the primary).
  await page.getByTestId('design-prompt-more').click();
  await page.locator('.ctx-menu').getByRole('menuitem', { name: /Draft only/ }).click();

  // The draft opens in the artifact view at v1.
  await expect(page).toHaveURL(/#\/design\/a\//);
  await expect(page.getByTestId('design-stage')).toBeVisible();
  // History lives in the side panel's Versions tab.
  await page.getByTestId('design-tab-versions').click();
  const chips = page.getByTestId('design-version-row');
  await expect(chips).toHaveCount(1);

  // Edit the HTML source; it autosaves → v2.
  await page.getByTestId('design-source-toggle').click();
  const editor = page.locator('.cm-content').first();
  await editor.click();
  await page.keyboard.press('ControlOrMeta+End');
  await page.keyboard.type('\n<!-- e2e edit -->');
  await expect(page.getByTestId('design-dirty')).toBeVisible();
  await expect(page.getByTestId('design-save-state')).toHaveAttribute('data-state', 'saved');
  await expect(chips).toHaveCount(2);
  await expect(page.getByTestId('design-dirty')).toHaveCount(0);

  // View v1 on its own, compare it with the current version, then Restore v1 (asks first).
  await chips.filter({ hasText: 'v1' }).first().locator('button.row-main').click();
  await expect(page.getByTestId('design-viewing')).toContainText('Viewing v1');
  await page.getByTestId('design-version-compare').click();
  const cmp = page.getByTestId('design-compare-modal');
  await expect(cmp).toBeVisible();
  await page.getByRole('tab', { name: 'Changes' }).click();
  await expect(cmp).toContainText('e2e edit');
  await page.getByTestId('design-restore').click();
  const confirm = page.getByRole('dialog', { name: 'Restore version' });
  await expect(confirm).toContainText('nothing is lost');
  // Restore runs server-side (no download + base64 re-upload of the bytes).
  const restoreReq = page.waitForRequest((r) => r.method() === 'POST' && /\/design\/artifacts\/[^/]+\/versions\/[^/]+\/restore$/.test(new URL(r.url()).pathname));
  await confirm.getByRole('button', { name: 'Restore v1' }).click();
  await restoreReq;
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
  await page.getByTestId('design-tab-links').click();
  await page.getByTestId('design-find-references').click();
  await page.getByTestId('design-ref-search').fill('Tier card');
  await expect(page.getByTestId('design-ref-hit').filter({ hasText: TARGET_TITLE })).toBeVisible();
  // Adding it as a reference marks it as referenced (a pinned `references` link).
  await page.getByTestId('design-ref-hit').filter({ hasText: TARGET_TITLE }).getByTestId('design-ref-add').click();
  await expect(page.getByTestId('design-ref-hit').filter({ hasText: TARGET_TITLE })).toContainText('Referenced');
});

test('a project page loads its own designs from the server (not the lobby library)', async ({ page }) => {
  const updates: string[] = [];
  page.on('websocket', (socket) => {
    if (!socket.url().includes('/ws/events')) return;
    socket.on('framereceived', ({ payload }) => {
      const event = JSON.parse(String(payload));
      if (event.type === 'design_artifact_updated' && event.change === 'meta') updates.push(event.artifact_id);
    });
  });
  const { ctx, base } = await apiCtx();
  const project = await postJson(ctx, `${base}${V1}/design/projects`, { workspace_id: wsId, name: `Scoped page ${stamp}` });
  const titles = [`Scoped A ${stamp}`, `Scoped B ${stamp}`];
  const artifactIds: string[] = [];
  for (const title of titles) {
    const created = await postJson(ctx, `${base}${V1}/design/artifacts`, {
      workspace_id: wsId,
      project_id: project.id,
      format: 'd2',
      title,
      content: 'a -> b\n',
    });
    artifactIds.push(created.artifact.id);
  }
  await ctx.dispose();

  // The page asks /design/search scoped to the project.
  const scoped = page.waitForRequest((r) => r.url().includes('/design/search') && r.url().includes(`project_id=${project.id}`));
  await openDesign(page, `design/p/${project.id}`);
  await scoped;
  for (const title of titles) await expect(page.getByText(title, { exact: true })).toBeVisible();
  await expect(page.getByTestId('design-collection-more')).toHaveCount(0); // 2 < one page
  await expectNoHorizontalOverflow(page);

  // A live rename PATCHES that one card (a detail GET) — it no longer
  // re-reads the whole slice through /design/search. The rename only reaches
  // the page over /ws/events, so wait for the stream before sending it (a
  // first connect doesn't resync — an event sent before it is simply missed).
  await expect(page.locator('[title="Event stream: live"]')).toBeVisible({ timeout: 15_000 });
  let searches = 0;
  page.on('request', (r) => {
    if (r.url().includes('/design/search') && r.url().includes(`project_id=${project.id}`)) searches++;
  });
  const { ctx: ctx2, base: base2 } = await apiCtx();
  // Hold the browser's debounce clock while two REAL backend events arrive in
  // separate turns. The second event must not cancel the first card's timer.
  await page.clock.install();
  await page.clock.pauseAt(new Date());
  const renamed = [`Scoped renamed A ${stamp}`, `Scoped renamed B ${stamp}`];
  for (const [i, id] of artifactIds.entries()) {
    const response = await ctx2.patch(`${base2}${V1}/design/artifacts/${id}`, { data: { title: renamed[i] } });
    expect(response.ok(), await response.text()).toBeTruthy();
    expect(await response.json()).toMatchObject({ id, workspace_id: wsId, project_id: project.id, title: renamed[i] });
    await expect.poll(() => updates.includes(id), { message: `live rename event for ${id}` }).toBe(true);
    await page.clock.runFor(0); // flush the event's reactive turn without expiring its debounce
  }
  await ctx2.dispose();
  await page.clock.runFor(350);
  for (const title of renamed) await expect(page.getByText(title, { exact: true })).toBeVisible();
  expect(searches, 'a meta update patches the card, no slice reload').toBe(0);
});
