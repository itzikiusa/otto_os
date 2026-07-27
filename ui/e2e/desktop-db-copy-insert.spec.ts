import { test, expect, type Page, type Locator } from '@playwright/test';
import { apiCtx, seedWorkspace, seedDockerConnection } from './seed';

// ─────────────────────────────────────────────────────────────────────────────
// DB Explorer — "Copy as INSERT" across engines, against the seeded Docker stack.
//
// Regression (2026-07-27): the action was welded to `editable`, which requires a
// resolvable PRIMARY KEY because writes have to target an existing row. Copying
// rows out only needs a table name plus the values already on screen, so that
// gate was wrong — it hid the action for every ClickHouse result whose key
// columns aren't projected, and the generator itself was engine-blind, emitting
// SQL `INSERT INTO` even on MongoDB.
//
// This spec proves both halves:
//   • per-engine OUTPUT — ClickHouse gets SQL, Mongo gets `insertMany`
//   • the UNGATING — the action is offered on results that are explicitly NOT
//     editable (PK columns absent / `_id` projected away)
//
// Desktop-browser project only (needs a real right-click + the 3-pane layout).
// ─────────────────────────────────────────────────────────────────────────────

let workspaceId = '';
const conn: Record<'mongodb' | 'clickhouse', string | null> = {
  mongodb: null,
  clickhouse: null,
};

test.beforeAll(async () => {
  test.setTimeout(120_000);
  const { ctx, base } = await apiCtx();
  workspaceId = await seedWorkspace(ctx, base);
  for (const k of ['mongodb', 'clickhouse'] as const) {
    try {
      conn[k] = await seedDockerConnection(ctx, base, workspaceId, k);
    } catch {
      conn[k] = null;
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

async function runQuery(page: Page, sql: string): Promise<void> {
  const content = page.locator('.qe-edit .cm-content');
  await content.click();
  await page.keyboard.press('ControlOrMeta+a');
  await page.keyboard.press('Delete');
  await content.pressSequentially(sql, { delay: 6 });
  await page.keyboard.press('Escape'); // dismiss autocomplete
  await page.locator('.btn.small.primary', { hasText: 'Run' }).first().click();
  await expect(page.locator('.grid tbody tr:not(.spacer)').first()).toBeVisible({
    timeout: 20_000,
  });
}

/** First data cell of the first row (skipping the leading #/checkbox column). */
function firstCell(page: Page): Locator {
  return page.locator('.grid tbody tr:not(.spacer)').first().locator('td').nth(1);
}

/** Right-click the first row and run "Copy row as INSERT"; returns the text of
 *  the tab it opens (the statement is written into a NEW tab, never run). */
async function copyRowAsInsert(page: Page): Promise<string> {
  const cell = firstCell(page);
  await cell.scrollIntoViewIfNeeded();
  await cell.click({ button: 'right' });
  await expect(page.locator('.ctx-menu')).toBeVisible();
  const item = page.locator('.ctx-item', { hasText: /Copy row as INSERT/ });
  await expect(item, '"Copy row as INSERT" offered on the row menu').toHaveCount(1);
  await item.first().click();
  await expect(page.locator('.ctx-menu')).toBeHidden();
  // The generated statement lands in a freshly opened editor tab.
  let text = '';
  await expect
    .poll(
      async () => {
        text = ((await page.locator('.qe-edit .cm-content').textContent()) ?? '').replace(
          / /g,
          ' ',
        );
        return text;
      },
      { timeout: 15_000 },
    )
    .toMatch(/INSERT|insertMany/);
  return text;
}

test.describe('ClickHouse', () => {
  test('emits SQL INSERT, and is offered even when no primary key resolves', async ({ page }) => {
    test.skip(!conn.clickhouse, 'ClickHouse container not reachable');
    await openConn(page, 'e2e-clickhouse');

    // 1) Full row — the key columns (event_type, ts) are projected, so the grid
    //    is editable too. Output must be ClickHouse-quoted, db-qualified SQL.
    await runQuery(page, 'SELECT * FROM analytics.events LIMIT 5');
    const full = await copyRowAsInsert(page);
    expect(full).toContain('INSERT INTO `analytics`.`events`');
    expect(full).toContain('VALUES');
    expect(full.trim().endsWith(';'), 'SQL statement is terminated').toBe(true);

    // 2) The ungating: project columns that EXCLUDE the MergeTree key, so the
    //    result can't be edited — copying rows out must still work. This is the
    //    exact case the old `editable` gate silently removed the action for.
    await runQuery(page, 'SELECT event_id, path FROM analytics.events LIMIT 5');
    await expect(
      page.locator('.gt-edit-hint'),
      'result is NOT editable without the key columns',
    ).toHaveCount(0);
    const projected = await copyRowAsInsert(page);
    expect(projected).toContain('INSERT INTO `analytics`.`events`');
    expect(projected).toContain('`event_id`');
    expect(projected, 'only the projected columns are emitted').not.toContain('`event_type`');
  });
});

test.describe('MongoDB', () => {
  test('emits insertMany — never SQL — and survives _id being projected away', async ({ page }) => {
    test.skip(!conn.mongodb, 'MongoDB container not reachable');
    await openConn(page, 'e2e-mongodb');

    // 1) A plain find: editable (has `_id`). Output must be Mongo syntax.
    await runQuery(page, 'db.orders.find({})');
    const docs = await copyRowAsInsert(page);
    expect(docs, 'Mongo must not get SQL').not.toContain('INSERT INTO');
    expect(docs).toContain('db.orders.insertMany([');
    expect(docs).toContain('"status"');
    // The seeded `_id`s are plain numbers, so they stay numbers — the `$oid`
    // wrapper is only for 24-hex ObjectId strings.
    expect(docs).toMatch(/"_id":\s*1\b/);

    // 2) The ungating: project `_id` away → not editable, still copyable.
    await runQuery(page, 'db.orders.find({}, {customerId: 1, status: 1, _id: 0})');
    await expect(
      page.locator('.gt-edit-hint'),
      'result is NOT editable once _id is projected away',
    ).toHaveCount(0);
    const noId = await copyRowAsInsert(page);
    expect(noId).toContain('db.orders.insertMany([');
    expect(noId).not.toContain('INSERT INTO');
    expect(noId, '_id was projected away, so it must not appear').not.toContain('"_id"');
  });
});
