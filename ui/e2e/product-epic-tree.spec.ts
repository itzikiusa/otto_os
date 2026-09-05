import { test, expect, type Page } from '@playwright/test';
import { apiCtx, seedWorkspace } from './seed';

// ── E2E: Product epic tree (docs/design/product-design-arena.md §3.2) ──────────
//
// Seeds an EPIC with two children in two folders through the API, then asserts
// the sidebar renders the tree (epic row with a child count, collapsible folder
// headers, indented children), that the breadcrumb `Epic › Folder › Title` shows
// for a child, that the epic Overview has the Children board, that a top-level
// draft can be moved under the epic via the row context menu and STAYS there
// after a reload (server round-trip via PATCH parent_id), and that a `doc` child
// hides the analysis/plan tabs.
//
// Desktop-width viewport so the two-pane layout (list + content) renders rather
// than the ≤640px accordion. Product stories are GLOBAL across workspaces and the
// suite runs in parallel against ONE shared daemon, so every title is unique.
test.use({ viewport: { width: 1280, height: 900 }, actionTimeout: 12_000 });

const RUN = `${Date.now()}-${Math.random().toString(36).slice(2, 8)}`;
const EPIC_TITLE = `E2E Epic ${RUN}`;
const CHILD_A = `E2E Tier ladder ${RUN}`;
const CHILD_B = `E2E Feature draft ${RUN}`;
const LOOSE = `E2E Loose story ${RUN}`;
const DOC_CHILD = `E2E Design note ${RUN}`;

let workspaceId = '';
let epicId = '';
let looseId = '';

async function must(r: { ok(): boolean; status(): number; text(): Promise<string>; json(): Promise<any> }, what: string) {
  if (!r.ok()) throw new Error(`${what} → ${r.status()} ${await r.text()}`);
  return r.json();
}

test.beforeAll(async () => {
  const { ctx, base } = await apiCtx();
  workspaceId = await seedWorkspace(ctx, base);
  // The epic: a draft flagged `tree_kind:'epic'` (a Jira story with children would
  // render the same — the UI treats "has children" as epic too).
  const epic = await must(
    await ctx.post(`${base}/api/v1/workspaces/${workspaceId}/product/drafts`, { data: { title: EPIC_TITLE } }),
    'create epic draft',
  );
  epicId = epic.story.id;
  await must(await ctx.patch(`${base}/api/v1/product/stories/${epicId}`, { data: { tree_kind: 'epic' } }), 'mark epic');
  // Two children in two folders via the children endpoint.
  await must(
    await ctx.post(`${base}/api/v1/product/stories/${epicId}/children`, {
      data: { title: CHILD_A, tree_kind: 'story', folder: 'Design' },
    }),
    'create child A',
  );
  await must(
    await ctx.post(`${base}/api/v1/product/stories/${epicId}/children`, {
      data: { title: CHILD_B, tree_kind: 'story', folder: 'PO' },
    }),
    'create child B',
  );
  // A doc child (lightweight: hides analysis/plan tabs).
  await must(
    await ctx.post(`${base}/api/v1/product/stories/${epicId}/children`, {
      data: { title: DOC_CHILD, tree_kind: 'doc', folder: 'Design' },
    }),
    'create doc child',
  );
  // A loose top-level draft to move under the epic from the UI.
  const loose = await must(
    await ctx.post(`${base}/api/v1/workspaces/${workspaceId}/product/drafts`, { data: { title: LOOSE } }),
    'create loose draft',
  );
  looseId = loose.story.id;
  await ctx.dispose();
});

test.beforeEach(async ({ page }) => {
  await page.addInitScript((wsId) => {
    localStorage.setItem('otto_workspace', wsId as string);
    localStorage.setItem('otto_rail_expanded', '0');
  }, workspaceId);
});

async function openProduct(page: Page): Promise<void> {
  await page.goto('/#/product');
  await expect(page.locator('.product-page')).toBeVisible({ timeout: 30_000 });
  await page.waitForLoadState('networkidle').catch(() => {});
  await expect(page.locator('.story-row', { hasText: EPIC_TITLE }).first()).toBeVisible({ timeout: 20_000 });
}

function epicWrap(page: Page) {
  return page.locator('.story-row-wrap.epic', { has: page.locator('.story-title', { hasText: EPIC_TITLE }) }).first();
}

test('epic tree: epic row, folders and indented children render; the epic collapses', async ({ page }) => {
  test.setTimeout(60_000);
  await openProduct(page);

  const epic = epicWrap(page);
  await expect(epic).toBeVisible();
  // "epic · 3" badge (two stories + one doc).
  await expect(epic.locator('.epic-badge')).toHaveText(/epic · 3/);

  // Folder headers + children (indented rows carry the `child` class).
  await expect(page.locator('.folder-head', { hasText: 'Design/' })).toBeVisible();
  await expect(page.locator('.folder-head', { hasText: 'PO/' })).toBeVisible();
  const childA = page.locator('.story-row-wrap.child', { hasText: CHILD_A });
  const childB = page.locator('.story-row-wrap.child', { hasText: CHILD_B });
  await expect(childA).toBeVisible();
  await expect(childB).toBeVisible();
  await expect(page.locator('.story-row-wrap.child', { hasText: DOC_CHILD }).locator('.draft-badge.doc')).toHaveText('DOC');

  // Collapse the epic → children + folders disappear; expand → back.
  await epic.locator('.tree-toggle').click();
  await expect(childA).toBeHidden();
  await expect(page.locator('.folder-head', { hasText: 'PO/' })).toBeHidden();
  await epic.locator('.tree-toggle').click();
  await expect(childA).toBeVisible();

  // Collapse just the Design folder → only its children hide.
  await page.locator('.folder-head', { hasText: 'Design/' }).click();
  await expect(childA).toBeHidden();
  await expect(childB).toBeVisible();
});

test('epic tree: child breadcrumb + epic Overview children board + Add child menu', async ({ page }) => {
  test.setTimeout(60_000);
  await openProduct(page);

  // Open a child → breadcrumb `Epic › Folder › Title`.
  await page.locator('.story-row', { hasText: CHILD_A }).first().click();
  const crumbs = page.locator('.crumbs');
  await expect(crumbs).toBeVisible({ timeout: 15_000 });
  await expect(crumbs.locator('.crumb').first()).toContainText(EPIC_TITLE);
  await expect(crumbs).toContainText('Design');
  await expect(crumbs.locator('.crumb.cur')).toContainText(CHILD_A);

  // Clicking the epic crumb opens the epic; its Overview shows the Children board.
  await crumbs.locator('.crumb').first().click();
  await expect(page.locator('.overview')).toBeVisible({ timeout: 15_000 });
  const board = page.locator('.children-board');
  await expect(board).toBeVisible({ timeout: 15_000 });
  await expect(board.locator('.cb-col-head', { hasText: 'Design' })).toBeVisible();
  await expect(board.locator('.cb-col-head', { hasText: 'PO' })).toBeVisible();
  await expect(board.locator('.cb-card', { hasText: CHILD_A })).toBeVisible();
  await expect(board.locator('.cb-rollup')).toContainText('3 total');

  // The header offers Add child ▾ (Story · Doc) for an epic.
  await page.locator('.add-child-btn').click();
  await expect(page.getByRole('menuitem', { name: /^Story/ })).toBeVisible();
  await expect(page.getByRole('menuitem', { name: /^Doc/ })).toBeVisible();
  await page.keyboard.press('Escape');
});

test('epic tree: Move to epic… via the row context menu persists after reload', async ({ page }) => {
  test.setTimeout(90_000);
  await openProduct(page);

  const loose = page.locator('.story-row-wrap', { has: page.locator('.story-title', { hasText: LOOSE }) }).first();
  await expect(loose).toBeVisible();
  await expect(loose).not.toHaveClass(/child/);

  // Right-click → Move to epic… → filterable picker → our epic.
  await loose.locator('.story-row').click({ button: 'right' });
  await page.getByRole('menuitem', { name: 'Move to epic…' }).click();
  const menu = page.locator('.ctx-menu');
  await expect(menu).toBeVisible();
  await menu.locator('.ctx-search-input').fill(EPIC_TITLE);
  await page.getByRole('menuitem', { name: new RegExp(EPIC_TITLE) }).click();

  // The row is now an indented child of the epic (epic count 4).
  const moved = page.locator('.story-row-wrap.child', { hasText: LOOSE });
  await expect(moved).toBeVisible({ timeout: 15_000 });
  await expect(epicWrap(page).locator('.epic-badge')).toHaveText(/epic · 4/);

  // Reload → still under the epic (server round-trip, not optimistic state).
  await page.reload();
  await openProduct(page);
  await expect(page.locator('.story-row-wrap.child', { hasText: LOOSE })).toBeVisible({ timeout: 20_000 });

  // And the API agrees.
  const { ctx, base } = await apiCtx();
  const detail = await (await ctx.get(`${base}/api/v1/product/stories/${looseId}`)).json();
  expect(detail.story.parent_id).toBe(epicId);
  await ctx.dispose();
});

test('epic tree: a doc child hides the analysis / plan tabs', async ({ page }) => {
  test.setTimeout(60_000);
  await openProduct(page);
  await page.locator('.story-row', { hasText: DOC_CHILD }).first().click();
  await expect(page.locator('.overview')).toBeVisible({ timeout: 15_000 });
  // Story group still there (Overview / Rewrite / Design)…
  await page.getByRole('tab', { name: 'Story', exact: true }).click();
  await expect(page.locator('.tab-strip .st', { hasText: 'Design' })).toBeVisible();
  // …but the Discover group has no Analysis sub-tab and Deliver is gone entirely.
  await page.getByRole('tab', { name: 'Discover', exact: true }).click();
  await expect(page.locator('.sub-tab-strip .st', { hasText: 'Analysis' })).toHaveCount(0);
  await expect(page.getByRole('tab', { name: 'Deliver', exact: true })).toHaveCount(0);
});
