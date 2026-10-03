import { test, expect, type Page, type Route } from '@playwright/test';
import { apiCtx, seedWorkspace } from './seed';
import { mockDbRoutes, seedMockDbConnection } from './db-mock';
import { expectFullyInViewport, expectNoHorizontalOverflow } from './helpers';

// ─────────────────────────────────────────────────────────────────────────────
// DB Explorer — "Run on…" (multi-target / parameterised runs). Two real mock
// profiles on the throwaway daemon (stg + PROD, both at loopback port 9); the
// engine calls and the multi-run endpoints are answered by `page.route`, so
// this exercises the UI flow end to end: pick targets + values → preview of
// every final statement → typed production confirmation → live results.
// ─────────────────────────────────────────────────────────────────────────────

const suffix = Math.random().toString(36).slice(2, 8);
const STG = `mr-stg-${suffix}`;
const PROD = `mr-prod-${suffix}`;
let workspaceId = '';
let stgId = '';
let prodId = '';

test.beforeAll(async () => {
  const { ctx, base } = await apiCtx();
  workspaceId = await seedWorkspace(ctx, base);
  stgId = await seedMockDbConnection(ctx, base, workspaceId, STG);
  prodId = await seedMockDbConnection(ctx, base, workspaceId, PROD);
  const r = await ctx.patch(`${base}/api/v1/connections/${prodId}`, {
    data: { name: PROD, kind: 'mysql', params: { host: '127.0.0.1', port: 9, user: 'mock', db: 'shop' }, environment: 'prod' },
  });
  expect(r.ok(), await r.text()).toBeTruthy();
  await ctx.dispose();
});

test.beforeEach(async ({ page }, info) => {
  test.skip(info.project.name !== 'desktop-browser', 'Desktop flow');
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.addInitScript((id) => {
    localStorage.setItem('otto_workspace', id);
    localStorage.setItem('otto_rail_expanded', '0');
    localStorage.removeItem(`otto_db_open:${id}`);
  }, workspaceId);
});

const STATEMENT = 'UPDATE orders SET region = :region WHERE brand_id = :brand';

function plan() {
  const targets = [
    { index: 0, connection_id: stgId, connection_name: STG, environment: 'staging', read_only: false, guarded: false, node: 'shop', label: `${STG} · shop` },
    { index: 1, connection_id: prodId, connection_name: PROD, environment: 'prod', read_only: false, guarded: true, node: 'shop', label: `${PROD} · shop` },
  ];
  const runs = [];
  let i = 0;
  for (const t of targets) {
    for (const b of ['1', '2']) {
      runs.push({
        index: i++,
        target: t.index,
        values: [{ name: 'region', value: 'eu' }, { name: 'brand', value: b }],
        label: `${t.label} · region=eu, brand=${b}`,
        statement: `UPDATE orders SET region = 'eu' WHERE brand_id = ${b}`,
        is_write: true,
        needs_confirm: t.guarded,
      });
    }
  }
  return { engine: 'mysql', placeholders: ['region', 'brand'], targets, runs, write_count: 4, needs_confirm: true, warnings: [], plan_hash: 'h1' };
}

async function fixture(page: Page) {
  const starts: { confirm: boolean; hash: string | null }[] = [];
  for (const id of [stgId, prodId]) await mockDbRoutes(page, id);
  const p = plan();
  let polls = 0;
  const job = (status: string) => ({
    id: 'job1',
    status: status === 'running' ? 'running' : 'done',
    engine: 'mysql',
    created_at: new Date().toISOString(),
    concurrency: 1,
    stop_on_error: true,
    read_only: false,
    statement_preview: STATEMENT,
    summary:
      status === 'running'
        ? { total: 4, ok: 1, failed: 0, running: 1, pending: 2, skipped: 0, cancelled: 0 }
        : { total: 4, ok: 3, failed: 1, running: 0, pending: 0, skipped: 0, cancelled: 0 },
    targets: p.targets,
    items: p.runs.map((r, i) => ({
      index: r.index,
      target: r.target,
      label: r.label,
      values: r.values,
      statement_preview: r.statement,
      is_write: true,
      status: status === 'running' ? ['ok', 'running', 'pending', 'pending'][i] : i === 3 ? 'failed' : 'ok',
      duration_ms: 12,
      rows_affected: i === 3 ? undefined : 7,
      error: status !== 'running' && i === 3 ? 'Table shop.orders is read only' : undefined,
      has_result: i !== 3,
    })),
  });
  await page.route('**/api/v1/db/multi-run/plan', (route: Route) => route.fulfill({ json: p }));
  await page.route('**/api/v1/db/multi-runs', (route: Route) => {
    if (route.request().method() === 'GET') return route.fulfill({ json: [] });
    const body = route.request().postDataJSON();
    starts.push({ confirm: body.confirm_write === true, hash: body.plan_hash ?? null });
    return route.fulfill({ status: 202, json: job('running') });
  });
  // `*`: the poll carries `?since=<seq>` (the mock answers in full).
  await page.route('**/api/v1/db/multi-runs/job1*', (route: Route) => {
    polls++;
    return route.fulfill({ json: job(polls > 1 ? 'done' : 'running') });
  });
  await page.route('**/api/v1/db/multi-runs/job1/items/*', (route: Route) => {
    const index = Number(route.request().url().split('/').pop());
    return route.fulfill({
      json: {
        item: job('done').items[index],
        connection_id: p.targets[p.runs[index].target].connection_id,
        node: 'shop',
        statement: p.runs[index].statement,
        result: { columns: [{ name: 'result' }], rows: [['7 rows affected']], stats: { duration_ms: 12, row_count: 1 }, truncated: false, rows_affected: 7 },
      },
    });
  });
  return { starts };
}

async function openStg(page: Page): Promise<void> {
  await page.goto('/#/database');
  await expect(page.locator('.shell')).toBeVisible({ timeout: 30_000 });
  const picker = page.locator('.side-switch .ss', { hasText: 'Connections' }).first();
  if (await picker.isVisible()) await picker.click();
  await page.locator('.conn-list .conn-name', { hasText: STG }).first().click();
  await expect(page.locator('.query-editor')).toBeVisible();
  const editor = page.locator('.query-editor .qe-edit .cm-content');
  await editor.click();
  await page.keyboard.press('ControlOrMeta+A');
  await page.keyboard.insertText(STATEMENT);
}

for (const scheme of ['light', 'dark'] as const) {
  test(`run on stg + prod × two brands with a typed production confirm (${scheme})`, async ({ page }, info) => {
    await page.addInitScript((value) => localStorage.setItem('otto_scheme', value), scheme);
    const { starts } = await fixture(page);
    await openStg(page);

    await page.getByTestId('multi-run-open').click();
    const sheet = page.getByTestId('multi-run');
    await expect(sheet).toBeVisible();

    // Targets: stg is pre-picked; add prod and its `shop` database on both.
    await sheet.locator('.mr-conn', { hasText: PROD }).locator('input[type=checkbox]').first().check();
    for (const name of [STG, PROD]) {
      await sheet.locator('.mr-conn', { hasText: name }).locator('.mr-scope', { hasText: 'shop' }).locator('input').check();
    }
    // Parameters: region fixed, brand swept.
    await sheet.getByLabel('Values for region').fill('eu');
    await sheet.getByLabel('Values for brand').fill('1, 2');
    await page.getByRole('button', { name: 'Preview 4 runs' }).click();

    // Preview: every final statement, flagged.
    await expect(sheet.locator('.mr-runs > li')).toHaveCount(4);
    await expect(sheet).toContainText("UPDATE orders SET region = 'eu' WHERE brand_id = 2");
    await expect(sheet.locator('.mr-tag.danger')).toHaveCount(2);
    await expectNoHorizontalOverflow(page);
    await expectFullyInViewport(page, page.locator('.sheet[role="dialog"]').first(), 'multi-run sheet');

    // Confirm: the prod runs are listed; Run stays disabled until the phrase.
    await page.getByRole('button', { name: /Review 2 guarded writes/ }).click();
    await expect(sheet.locator('.mr-guarded')).toHaveCount(2);
    const run = page.getByRole('button', { name: 'Run 4' });
    await expect(run).toBeDisabled();
    await sheet.getByLabel(`Type ${PROD} to confirm`).fill(PROD);
    await expect(run).toBeEnabled();
    const shot = info.outputPath(`multirun-confirm-${scheme}.png`);
    await page.screenshot({ path: shot });
    await info.attach(`multirun-confirm-${scheme}`, { path: shot, contentType: 'image/png' });
    await run.click();

    // Results: live summary, then the failed run's error and a result.
    await expect(sheet.locator('.mr-status')).toContainText('3 ok · 1 failed', { timeout: 10_000 });
    await expect(sheet).toContainText('Table shop.orders is read only');
    await sheet.locator('.mr-res-row').first().click();
    await expect(sheet.locator('.mr-table')).toContainText('7 rows affected');
    expect(starts).toEqual([{ confirm: true, hash: 'h1' }]);
  });
}
