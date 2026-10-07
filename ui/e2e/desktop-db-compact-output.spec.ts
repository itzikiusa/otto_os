import { test, expect, type Page } from '@playwright/test';
import { apiCtx, seedWorkspace } from './seed';
import { mockDbRoutes, seedMockDbConnection } from './db-mock';
import { expectFullyInViewport, expectNoHorizontalOverflow } from './helpers';

// Real isolated profile + real workbench; engine calls stay behind db-mock's
// closed-loopback fixture. No Docker, live database or production state needed.
let workspaceId = '';
let connId = '';
const connectionName = `compact-output-${Math.random().toString(36).slice(2, 8)}`;

test.beforeAll(async () => {
  const { ctx, base } = await apiCtx();
  try {
    workspaceId = await seedWorkspace(ctx, base);
    connId = await seedMockDbConnection(ctx, base, workspaceId, connectionName);
  } finally { await ctx.dispose(); }
});

test.beforeEach(async ({ page }, testInfo) => {
  test.skip(testInfo.project.name !== 'desktop-browser', 'explicit viewport matrix on desktop-browser');
  await page.addInitScript(wsId => {
    localStorage.setItem('otto_workspace', wsId);
    localStorage.setItem('otto_rail_expanded', '0');
    localStorage.setItem('otto_db_row_limit', '100');
  }, workspaceId);
  await mockDbRoutes(page, connId);
});

async function openQuery(page: Page, scheme: 'light' | 'dark'): Promise<void> {
  await page.emulateMedia({ colorScheme: scheme, reducedMotion: 'reduce' });
  await page.addInitScript(value => localStorage.setItem('otto_scheme', value), scheme);
  await page.goto('/#/database');
  await expect(page.locator('.shell')).toBeVisible();
  await page.locator(`.conn-row[data-connection-id="${connId}"] .conn-name`).click();
  const editor = page.locator('.qe-edit .cm-content');
  await expect(editor).toBeVisible();
  await editor.click();
  await page.keyboard.press('ControlOrMeta+A');
  await page.keyboard.insertText('SELECT * FROM shop.orders ORDER BY id');
  await page.keyboard.press('Escape');
}

for (const width of [390, 560]) for (const scheme of ['light', 'dark'] as const) {
  test(`compact results ${width}px ${scheme}: footer Next stays reachable and pages`, async ({ page }, testInfo) => {
    await page.setViewportSize({ width, height: 844 });
    const offsets: number[] = [];
    // Enough rows for real vertical virtualization and two full pages. Extra
    // footer statistics plus the real editability probe exercise the wrap.
    await page.route(new RegExp(`/connections/${connId}/db/query$`), async route => {
      const body = route.request().postDataJSON() as { offset?: number; statement: string };
      expect(body.statement).toBe('SELECT * FROM shop.orders ORDER BY id');
      const offset = body.offset ?? 0;
      offsets.push(offset);
      await route.fulfill({ json: {
        columns: [{ name: 'id', type_hint: 'BIGINT' }, { name: 'status', type_hint: 'VARCHAR(20)' }],
        rows: Array.from({ length: 100 }, (_, i) => [offset + i + 1, 'paid']),
        stats: { row_count: 100, duration_ms: 1234, bytes_read: 65536 },
        truncated: false, auto_limited: 100,
      } });
    });
    await openQuery(page, scheme);
    await page.locator('.btn.small.primary', { hasText: 'Run' }).first().click();
    const footer = page.locator('.grid-foot');
    await expect(footer.locator('.pg-range')).toHaveText('rows 1–100');
    await expect(footer.locator('.gt-edit-hint')).toBeVisible();
    const grid = page.locator('.grid-scroll');
    await grid.evaluate(el => { el.scrollTop = el.scrollHeight; });
    await expect.poll(() => grid.evaluate(el => el.scrollTop)).toBeGreaterThan(0);
    // Scroll the footer vertically into view, never the Next button sideways:
    // the assertion must catch a clipped pager, not scroll it out of trouble.
    await footer.scrollIntoViewIfNeeded();
    const next = footer.getByRole('button', { name: 'Next page', exact: true });
    await expectFullyInViewport(page, next, 'compact Next page');
    await expectFullyInViewport(page, footer.locator('.gt-edit-hint'), 'compact edit guidance');
    await expectNoHorizontalOverflow(page);
    // Bounding boxes alone miss ancestor clipping. Trial click proves the
    // button is actually hit-testable before keyboard activation.
    await next.click({ trial: true });
    await page.screenshot({ path: testInfo.outputPath(`db-footer-${width}-${scheme}.png`), animations: 'disabled' });
    await next.press('Enter');
    await expect(footer.locator('.pg-range')).toHaveText('rows 101–200');
    expect(offsets).toEqual([0, 100]);
    await expect(footer.getByRole('button', { name: 'Previous page', exact: true })).toBeEnabled();
    await expectNoHorizontalOverflow(page);
  });
}

for (const scheme of ['light', 'dark'] as const) {
  test(`first Explain ${scheme}: plan has visible output before any query ran`, async ({ page }, testInfo) => {
    await page.setViewportSize({ width: 1280, height: 800 });
    const seenQueries: string[] = [];
    await page.route(new RegExp(`/connections/${connId}/db/query$`), async route => {
      seenQueries.push(route.request().postData() ?? '');
      await route.fulfill({ status: 500, json: { message: 'fixture must not execute a query' } });
    });
    await page.route(new RegExp(`/connections/${connId}/db/query-plan$`), async route => {
      expect(route.request().postDataJSON().statement).toBe('SELECT * FROM shop.orders ORDER BY id');
      await route.fulfill({ json: {
        engine: 'mysql',
        root: { op: 'query_block', children: [{ op: 'ALL', object: 'orders', est_rows: 240, detail: '(shop.orders.total_cents > 5)', warnings: ['full table scan'], children: [] }] },
        raw: { table: 'orders', access_type: 'ALL', rows: 240 },
      } });
    });
    await openQuery(page, scheme);
    await page.getByRole('button', { name: 'Explain', exact: true }).click();
    const plan = page.locator('.plan-panel');
    await expect(plan).toBeVisible();
    await expect(page.getByText('No results yet', { exact: true })).toHaveCount(0);
    await expect.poll(() => page.locator('.qe-results').evaluate(el => el.getBoundingClientRect().height), 'Explain reserves output height').toBeGreaterThan(80);
    await expect.poll(() => plan.evaluate(el => el.getBoundingClientRect().height), 'plan owns enough height for its header and operation').toBeGreaterThan(60);
    await expectFullyInViewport(page, plan.locator('.plan-head'), 'first Explain header');
    await expectFullyInViewport(page, plan.locator('.plan-op').last(), 'first Explain operation');
    await expect(plan.locator('.plan-warn')).toHaveText('full table scan');
    await expectNoHorizontalOverflow(page);
    await page.screenshot({ path: testInfo.outputPath(`db-first-explain-${scheme}.png`), animations: 'disabled' });
    await plan.getByRole('button', { name: 'Raw JSON', exact: true }).click();
    await expect(plan.locator('.plan-raw')).toContainText('ALL');
    await plan.getByRole('button', { name: 'Close plan', exact: true }).click();
    await expect(plan).toHaveCount(0);
    await expect(page.getByText('No results yet', { exact: true })).toBeVisible();
    expect(seenQueries).toEqual([]);
  });
}
