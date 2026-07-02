import { test, expect, type Page, type Locator } from '@playwright/test';
import { apiCtx, seedWorkspace, seedDockerConnection } from './seed';

// ─────────────────────────────────────────────────────────────────────────────
// DB Explorer — Saved queries v2 + History usability (Task 7), driven against the
// live Docker MySQL:
//   • Save a query → reopen from the Saved list → the tab is LINKED, so the
//     toolbar button reads "Update" and Save PATCHes in place (no duplicate row).
//   • "Save as new" forks a second saved query.
//   • Inline rename in the Saved list.
//   • Saved search box filters by name AND statement.
//   • History: statements run from the UI appear, search filters them, and the
//     "Load more" control shows + grows the window when there are >100 rows.
//
// Desktop-browser project only; skips when MySQL is down.
// ─────────────────────────────────────────────────────────────────────────────

let workspaceId = '';
let mysqlConn: string | null = null;

// Distinct markers run from the UI in the history test (newest → top of the
// window); the filler rows below push the total over the 100-row page so the
// "Load more" control becomes active.
const HIST_ALPHA = "SELECT 'hist_alpha_marker' AS marker";
const HIST_BETA = "SELECT 'hist_beta_marker' AS marker";
const FILLERS = 110;

test.beforeAll(async () => {
  test.setTimeout(180_000);
  const { ctx, base } = await apiCtx();
  workspaceId = await seedWorkspace(ctx, base);
  try {
    mysqlConn = await seedDockerConnection(ctx, base, workspaceId, 'mysql');
  } catch {
    mysqlConn = null;
  }
  // Seed >100 history rows so the "Load more" pager is exercisable. Each /db/query
  // records one history row; run them in small batches to keep the pool happy.
  if (mysqlConn) {
    for (let start = 0; start < FILLERS; start += 20) {
      await Promise.all(
        Array.from({ length: Math.min(20, FILLERS - start) }, (_, k) =>
          ctx
            .post(`${base}/api/v1/connections/${mysqlConn}/db/query`, {
              data: { statement: `SELECT ${start + k} AS filler` },
            })
            .catch(() => {}),
        ),
      );
    }
  }
  await ctx.dispose().catch(() => {});
});

test.beforeEach(async ({ page }, testInfo) => {
  test.skip(testInfo.project.name !== 'desktop-browser', 'desktop-browser only');
  await page.addInitScript((wsId) => {
    localStorage.setItem('otto_workspace', wsId as string);
    localStorage.setItem('otto_rail_expanded', '0');
  }, workspaceId);
});

async function openConn(page: Page, name: string): Promise<void> {
  await page.goto('/#/database');
  await expect(page.locator('.shell')).toBeVisible({ timeout: 30_000 });
  const c = page.locator('.conn-list .conn-name', { hasText: name });
  await expect(c.first()).toBeVisible({ timeout: 30_000 });
  await c.first().click();
  await expect(page.locator('.main-tabs')).toBeVisible({ timeout: 20_000 });
}

// Type a statement into CodeMirror (replacing whatever's there), retry-until-text
// (CodeMirror can drop the leading char before focus settles).
async function typeStatement(page: Page, sql: string): Promise<void> {
  const content = page.locator('.qe-edit .cm-content');
  const want = sql.replace(/\s+/g, ' ').trim();
  for (let attempt = 0; attempt < 3; attempt++) {
    await content.click();
    await expect(content).toBeFocused({ timeout: 5_000 });
    await page.keyboard.press('ControlOrMeta+A');
    await page.keyboard.press('Delete');
    await content.pressSequentially(sql, { delay: 8 });
    await page.keyboard.press('Escape'); // dismiss any completion popup
    const got = ((await content.textContent()) ?? '').replace(/\s+/g, ' ').trim();
    if (got.startsWith(want)) return;
  }
  const got = ((await content.textContent()) ?? '').replace(/\s+/g, ' ').trim();
  expect(got, `editor should hold the statement (got: "${got}")`).toContain(want);
}

async function runStatement(page: Page, sql: string): Promise<void> {
  await typeStatement(page, sql);
  await page.locator('.btn.small.primary', { hasText: 'Run' }).first().click();
  await expect(page.locator('.btn.small.primary', { hasText: 'Run' }).first()).toBeVisible({
    timeout: 20_000,
  });
}

const toolbarSaveBtn = (page: Page): Locator =>
  page.locator('.qe-toolbar').getByRole('button', { name: /^(Save|Update)$/ });
const saveBar = (page: Page): Locator => page.locator('.save-bar');
const exactRe = (s: string) => new RegExp(`^${s.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')}$`);

/** Open the save bar, set a name, click the primary Save/Update. */
async function saveWith(page: Page, name: string | null): Promise<void> {
  await toolbarSaveBtn(page).click();
  await expect(saveBar(page)).toBeVisible();
  if (name !== null) await saveBar(page).locator('input').fill(name);
  await saveBar(page).getByRole('button', { name: /^(Save|Update)$/ }).click();
  await expect(saveBar(page)).toHaveCount(0);
}

/** Open the save bar and click "Save as new" (creates a fresh saved query). */
async function saveAsNew(page: Page, name: string): Promise<void> {
  await toolbarSaveBtn(page).click();
  await expect(saveBar(page)).toBeVisible();
  await saveBar(page).locator('input').fill(name);
  await saveBar(page).getByRole('button', { name: 'Save as new' }).click();
  await expect(saveBar(page)).toHaveCount(0);
}

async function openSideTab(page: Page, label: 'Saved' | 'History'): Promise<void> {
  await page.locator('.side-switch .ss', { hasText: label }).first().click();
}

const savedRows = (page: Page): Locator => page.locator('.saved-row');
const savedRow = (page: Page, name: string): Locator =>
  savedRows(page).filter({ has: page.locator('.ellipsis').filter({ hasText: exactRe(name) }) });
const histRow = (page: Page, needle: string): Locator =>
  page.locator('.hist-row').filter({ hasText: needle });

test('saved queries v2: save → update in place → save as new → rename → search', async ({
  page,
}) => {
  test.skip(!mysqlConn, 'mysql docker not reachable');
  await openConn(page, 'e2e-mysql');

  // 1) Save a first query.
  await typeStatement(page, 'SELECT 111 AS alpha_col');
  await saveWith(page, 'Alpha');
  await openSideTab(page, 'Saved');
  await expect(savedRows(page)).toHaveCount(1);
  await expect(savedRow(page, 'Alpha')).toBeVisible();

  // 2) Reopen from the Saved list → the tab is LINKED; the button reads "Update".
  await savedRow(page, 'Alpha').locator('.saved-open').click();
  await expect(toolbarSaveBtn(page)).toHaveText(/Update/);

  // 3) Edit the statement + Update in place → still ONE row (no duplicate).
  await typeStatement(page, 'SELECT 222 AS alpha_col_v2');
  await saveWith(page, null); // keep the name; PATCH the statement
  await openSideTab(page, 'Saved');
  await expect(savedRows(page)).toHaveCount(1, { timeout: 10_000 });

  // 4) "Save as new" from the linked tab → a SECOND saved query.
  await savedRow(page, 'Alpha').locator('.saved-open').click();
  await typeStatement(page, 'SELECT 333 AS beta_col');
  await saveAsNew(page, 'Beta');
  await openSideTab(page, 'Saved');
  await expect(savedRows(page)).toHaveCount(2);
  await expect(savedRow(page, 'Beta')).toBeVisible();

  // 5) Inline rename Beta → Gamma.
  await savedRow(page, 'Beta').getByRole('button', { name: 'Rename saved query' }).click();
  await page.locator('.rename-input').fill('Gamma');
  await page.locator('.rename-input').press('Enter');
  await expect(savedRow(page, 'Gamma')).toBeVisible();
  await expect(savedRow(page, 'Beta')).toHaveCount(0);

  // 6) Search by NAME.
  await page.locator('.list-search-input').fill('Gamma');
  await expect(savedRow(page, 'Gamma')).toBeVisible();
  await expect(savedRow(page, 'Alpha')).toHaveCount(0);

  // 7) Search by STATEMENT text (Alpha's statement holds `alpha_col_v2`).
  await page.locator('.list-search-input').fill('alpha_col_v2');
  await expect(savedRow(page, 'Alpha')).toBeVisible();
  await expect(savedRow(page, 'Gamma')).toHaveCount(0);
});

test('history: runs appear, search filters, Load more grows the window', async ({ page }) => {
  test.skip(!mysqlConn, 'mysql docker not reachable');
  await openConn(page, 'e2e-mysql');

  // Two distinct statements from the UI — newest, so they land in the window.
  await runStatement(page, HIST_ALPHA);
  await runStatement(page, HIST_BETA);
  await openSideTab(page, 'History');

  // Both appear.
  await expect(histRow(page, 'hist_alpha_marker')).toBeVisible({ timeout: 10_000 });
  await expect(histRow(page, 'hist_beta_marker')).toBeVisible();

  // Search filters (client-side over the loaded window).
  await page.locator('.list-search-input').fill('hist_alpha_marker');
  await expect(histRow(page, 'hist_alpha_marker')).toBeVisible();
  await expect(histRow(page, 'hist_beta_marker')).toHaveCount(0);
  await page.locator('.list-search-input').fill('');

  // "Load more" is shown (the window is full at 100 rows, >100 exist) and clicking
  // it grows the rendered list past the initial page.
  const loadMore = page.locator('.load-more');
  await expect(loadMore).toBeVisible({ timeout: 10_000 });
  const before = await page.locator('.hist-row').count();
  await loadMore.click();
  await expect.poll(() => page.locator('.hist-row').count(), { timeout: 15_000 }).toBeGreaterThan(
    before,
  );
});
