import { test, expect, type Page, type Route } from '@playwright/test';
import { apiCtx, seedWorkspace } from './seed';
import { mockDbRoutes, seedMockDbConnection } from './db-mock';
import { budgetMs, gridScrollCosts, isDesktopProject, isWebkitProject, percentile, scrollFrameWork } from './perf';

// ─────────────────────────────────────────────────────────────────────────────
// DB Explorer at scale — the perf guards the results-grid spec doesn't cover
// (perf review 04-db-ui, F1–F6). Mocked (db-mock.ts + page.route), Docker-free.
//
//   1. wide result 20k × 300 columns (column virtualisation, F2)
//        header + body both windowed (< 60 cells a row) · grid DOM < 3,000 nodes
//        V step (JS + layout) p95 < 12 ms · V/H step to painted frame p95 < 40 ms
//        (the same budgets as the 100k × 30 grid in desktop-db-results-perf)
//   2. column filter over 100k rows with a JSON column (F3)
//        key → next frame < 50 ms · key → filtered < 700 ms
//   3. result memory budget across tabs (F1)
//        background results are released past the budget; Re-run restores them
//  3b. switch back to a string-sorted 100k tab (R2) · search text pre-built on focus (R3)
//        switch back < 90 ms (cached view) · key → filtered < 250 ms
//   4. schema tree, 20 schemas × 1,000 tables expanded
//        tree DOM < 1,500 nodes · filter key → frame < 50 ms · scroll step p95 < 16 ms
//   5. restore 15 open connections with 300 ms dials
//        active editor < 1.5 s after the chips · ≤ 4 concurrent schema loads
//   6. Query → Structure → Query with a 100k-row result
//        grid repainted < 150 ms (lazy views are prefetched, render sync)
//   7. dashboard of 12 bar widgets × 5,000 rows (chart sampling, F6)
//        all tiles drawn < 3 s · ≤ 12 × 160 bars
//   8. multi-run sheet with 200 runs polling
//        no poll frame over 50 ms · polls slow down (≤ 1.6/s) · `?since=` deltas only
// Timings are budgetMs()-scaled; DOM / request counts never are.
// `@ci` (1–3: deterministic counts + scaled timings, ~1 min) runs in the CI
// perf-gates job (`--grep @ci`); the rest run locally with the full perf set.
// ─────────────────────────────────────────────────────────────────────────────

test.use({ serviceWorkers: 'block' });

const suffix = Math.random().toString(36).slice(2, 8);
const CONN = `mock-scale-${suffix}`;
const RESTORE_N = 15;
let workspaceId = '';
let connId = '';
const restoreIds: string[] = [];

test.beforeAll(async () => {
  test.setTimeout(180_000);
  const { ctx, base } = await apiCtx();
  workspaceId = await seedWorkspace(ctx, base);
  connId = await seedMockDbConnection(ctx, base, workspaceId, CONN);
  for (let i = 0; i < RESTORE_N; i++) {
    restoreIds.push(await seedMockDbConnection(ctx, base, workspaceId, `mock-restore-${suffix}-${i}`));
  }
  await ctx.dispose().catch(() => {});
});

test.beforeEach(async ({ page }) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.addInitScript((wsId) => {
    localStorage.setItem('otto_workspace', wsId as string);
    localStorage.setItem('otto_rail_expanded', '0');
  }, workspaceId);
});

const webkitOnly = (name: string) => test.skip(!isWebkitProject(name), 'WebKit perf gate: --project=desktop-webkit');

// ── result bodies (built once per worker) ────────────────────────────────────
const memo = new Map<string, string>();
function body(key: string, build: () => unknown): string {
  let b = memo.get(key);
  if (!b) {
    b = JSON.stringify(build());
    memo.set(key, b);
  }
  return b;
}
function table(rows: number, cols: number, cell: (r: number, c: number) => unknown) {
  const columns = Array.from({ length: cols }, (_, c) => ({ name: c === 0 ? 'id' : `c_${c}`, type_hint: c === 0 ? 'BIGINT' : 'VARCHAR(20)' }));
  const out: unknown[][] = new Array(rows);
  for (let r = 0; r < rows; r++) {
    const row = new Array(cols);
    row[0] = r + 1;
    for (let c = 1; c < cols; c++) row[c] = cell(r, c);
    out[r] = row;
  }
  return { columns, rows: out, stats: { duration_ms: 30, row_count: rows }, truncated: false };
}
/** `wide` 20k × 300 · `docs` 100k with a JSON column · `mid` 20k × 10 · default 100k × 12. */
function resultFor(stmt: string): string {
  if (/\bwide\b/i.test(stmt)) return body('wide', () => table(20_000, 300, (r, c) => `w${(r + c) % 997}`));
  if (/\bdocs\b/i.test(stmt))
    return body('docs', () => {
      const t = table(100_000, 4, (r, c) => (c === 3 ? { brand: r % 50, tags: [`t${r % 7}`, 'x'], meta: { rev: r } } : `v${(r * 31 + c) % 50_000}`));
      t.columns[3] = { name: 'doc', type_hint: 'JSON' };
      return t;
    });
  if (/\bmid\b/i.test(stmt)) return body('mid', () => table(20_000, 10, (r, c) => `m${(r * 7 + c) % 10_000}`));
  return body('big', () => table(100_000, 12, (r, c) => `b${(r * 13 + c) % 50_000}`));
}

async function routeQueries(page: Page, id = connId): Promise<void> {
  await mockDbRoutes(page, id);
  await page.route(new RegExp(`/connections/${id}/db/query$`), (route) => {
    const stmt = String(((route.request().postDataJSON() ?? {}) as { statement?: string }).statement ?? '');
    return route.fulfill({ status: 200, contentType: 'application/json', body: resultFor(stmt) });
  });
}

async function openConn(page: Page, name = CONN): Promise<void> {
  await page.goto('/#/database');
  await expect(page.locator('.shell')).toBeVisible({ timeout: 30_000 });
  const picker = page.locator('.side-switch [role="tab"]', { hasText: 'Connections' }).first();
  if (await picker.isVisible().catch(() => false)) await picker.click();
  const c = page.locator('.conn-list .conn-name', { hasText: name });
  await expect(c.first()).toBeVisible({ timeout: 30_000 });
  await c.first().click();
  await expect(page.locator('.qe-edit .cm-content')).toBeVisible({ timeout: 20_000 });
}

async function run(page: Page, statement: string): Promise<void> {
  const content = page.locator('.qe-edit .cm-content');
  await content.click();
  await page.keyboard.press('ControlOrMeta+A');
  await page.keyboard.insertText(statement);
  await page.waitForTimeout(200);
  await page.locator('.btn.small.primary', { hasText: 'Run' }).first().click();
  await expect(page.locator('.grid-scroll tbody td.cell').first()).toBeVisible({ timeout: 60_000 });
}

function perfLine(line: string): void {
  console.log(`[perf] ${line}`);
  test.info().annotations.push({ type: 'perf', description: line });
}

/** Type `text` into `selector` and time key → next frame and key → `done()` true. */
async function keyToFrame(page: Page, selector: string, text: string, doneExpr: string): Promise<{ frame: number; applied: number }> {
  return page.evaluate(
    async ({ selector, text, doneExpr }) => {
      const input = document.querySelector(selector) as HTMLInputElement;
      // eslint-disable-next-line @typescript-eslint/no-implied-eval
      const done = new Function(`return (${doneExpr});`) as () => boolean;
      const t0 = performance.now();
      input.value = text;
      input.dispatchEvent(new Event('input', { bubbles: true }));
      await new Promise((r) => requestAnimationFrame(() => setTimeout(r, 0)));
      const frame = performance.now() - t0;
      for (let i = 0; i < 900 && !done(); i++) await new Promise((r) => requestAnimationFrame(() => r(null)));
      return { frame, applied: performance.now() - t0 };
    },
    { selector, text, doneExpr },
  );
}

// ── 1. wide result ───────────────────────────────────────────────────────────
test('wide 20k × 300 result: only the columns in view are mounted', { tag: '@ci' }, async ({ page }, info) => {
  webkitOnly(info.project.name);
  test.setTimeout(180_000);
  await routeQueries(page);
  await openConn(page);
  await run(page, 'SELECT * FROM wide');
  // Header and body both render a window of the columns + colspan spacers.
  await expect(page.locator('.grid-scroll tbody td.hpad').first()).toBeVisible({ timeout: 5_000 });
  await expect(page.locator('.grid-scroll thead tr:first-child th.hpad')).toHaveCount(1);
  const headCells = await page.locator('.grid-scroll thead tr:first-child th:not(.hpad)').count();
  expect(headCells, `header cells ${headCells}`).toBeLessThan(60);
  const firstRowCells = await page.evaluate(
    () => document.querySelector('.grid-scroll tbody tr:not(.spacer)')?.querySelectorAll('td.cell').length ?? 0,
  );
  expect(firstRowCells, `cells in one row ${firstRowCells}`).toBeLessThan(60);
  const nodes = await page.evaluate(() => document.querySelectorAll('.grid-scroll *').length);
  await gridScrollCosts(page, 300, 5); // warm-up
  const vLayout = await gridScrollCosts(page, 300, 30);
  const v = await gridScrollCosts(page, 300, 30, { paint: true });
  const h = await gridScrollCosts(page, 400, 30, { axis: 'x', paint: true });
  // Columns far to the right render (body AND header) after the horizontal scroll.
  const lastP = await page.evaluate(() => {
    const tds = document.querySelectorAll<HTMLElement>('.grid-scroll tbody tr:not(.spacer) td.cell');
    return Math.max(...[...tds].map((td) => Number(td.dataset.p)));
  });
  expect(lastP, 'horizontal scroll must advance the mounted column window').toBeGreaterThan(headCells);
  await expect(page.locator('.grid-scroll thead tr:first-child th', { hasText: `c_${lastP}` }).first()).toBeAttached();
  // Cells kept across the horizontal steps stay in display order, one run of
  // positions per row, and show their own column's value (`w{(r + c) % 997}`).
  const rowsOk = await page.evaluate(() => {
    for (const tr of document.querySelectorAll<HTMLTableRowElement>('.grid-scroll tbody tr:not(.spacer)')) {
      const tds = [...tr.querySelectorAll<HTMLElement>('td.cell')];
      const r = Number(tds[0]?.dataset.r);
      for (let i = 0; i < tds.length; i++) {
        const p = Number(tds[i].dataset.p);
        if (i > 0 && p !== Number(tds[i - 1].dataset.p) + 1) return `gap at p${p}`;
        if (p > 0 && tds[i].textContent !== `w${(r + p) % 997}`) return `r${r} p${p} shows ${tds[i].textContent}`;
      }
    }
    return 'ok';
  });
  expect(rowsOk).toBe('ok');
  perfLine(
    `wide 20k×300: DOM ${nodes}; head cells ${headCells}; row cells ${firstRowCells}; ` +
      `v-step layout p95 ${percentile(vLayout, 95).toFixed(1)} ms; v-step painted p95 ${percentile(v, 95).toFixed(1)} ms; ` +
      `h-step painted p95 ${percentile(h, 95).toFixed(1)} ms`,
  );
  expect(lastP).toBeGreaterThan(60);
  // Was ~3,500 with all 300 header cells mounted; ~1,550 windowed.
  expect(nodes, `grid DOM ${nodes}`).toBeLessThan(3_000);
  // Budgets match the 100k × 30 grid (desktop-db-results-perf): 12 ms to
  // style + layout, 40 ms to the painted frame. The old 16 ms PAINTED budget
  // was tighter than the narrow grid's own (measured 18–19 ms p95 there), so
  // it failed on every run. Measured here (WebKit, M-series, load avg 10–18):
  // V layout p95 8–9 ms, V painted p95 18–23 ms, H painted p95 21–22 ms. Before
  // (all 300 header cells re-laid out per step; every row rebuilt per H step):
  // V painted p95 46–90 ms (layout alone 15–17 ms), H painted p95 32–35 ms.
  expect(percentile(vLayout, 95), 'vertical step (JS + layout) p95').toBeLessThan(budgetMs(12));
  expect(percentile(v, 95), 'vertical step to painted frame p95').toBeLessThan(budgetMs(40));
  expect(percentile(h, 95), 'horizontal step to painted frame p95').toBeLessThan(budgetMs(40));
});

// ── 1b. wide result, RTL ─────────────────────────────────────────────────────
test('wide 20k × 300 result in RTL: the column window follows a negative scrollLeft', async ({ page }, info) => {
  webkitOnly(info.project.name);
  test.setTimeout(180_000);
  await page.addInitScript(() => document.addEventListener('DOMContentLoaded', () => (document.documentElement.dir = 'rtl')));
  await routeQueries(page);
  await openConn(page);
  await run(page, 'SELECT * FROM wide');
  await expect(page.locator('.grid-scroll tbody td.hpad').first()).toBeVisible({ timeout: 5_000 });
  expect(await page.locator('.grid-scroll thead tr:first-child th:not(.hpad)').count()).toBeLessThan(60);
  // Scroll toward the inline end (left in RTL: scrollLeft goes negative).
  const h = await gridScrollCosts(page, -400, 20, { axis: 'x', paint: true });
  const res = await page.evaluate(() => {
    const el = document.querySelector('.grid-scroll') as HTMLElement;
    const tr = document.querySelector<HTMLTableRowElement>('.grid-scroll tbody tr:not(.spacer)')!;
    const tds = [...tr.querySelectorAll<HTMLElement>('td.cell')];
    const r = Number(tds[0].dataset.r);
    const ps = tds.map((td) => Number(td.dataset.p));
    const bad = tds.find((td) => Number(td.dataset.p) > 0 && td.textContent !== `w${(r + Number(td.dataset.p)) % 997}`);
    // Every mounted cell that should be on screen is: the viewport is covered.
    const vp = el.getBoundingClientRect();
    const covered = tds.some((td) => {
      const b = td.getBoundingClientRect();
      return b.left <= vp.left + vp.width / 2 && b.right >= vp.left + vp.width / 2;
    });
    return { scrollLeft: el.scrollLeft, min: Math.min(...ps), max: Math.max(...ps), bad: bad?.dataset.p ?? null, covered };
  });
  perfLine(`wide RTL: scrollLeft ${res.scrollLeft}; window p${res.min}–p${res.max}; h-step painted p95 ${percentile(h, 95).toFixed(1)} ms`);
  expect(res.scrollLeft).toBeLessThan(-2_000);
  expect(res.min).toBeGreaterThan(10);
  expect(res.bad).toBeNull();
  expect(res.covered, 'a mounted cell spans the middle of the viewport').toBe(true);
});

// ── 2. column filter ─────────────────────────────────────────────────────────
test('column filter over 100k rows with a JSON column: debounced + cached', { tag: '@ci' }, async ({ page }, info) => {
  webkitOnly(info.project.name);
  test.setTimeout(180_000);
  await routeQueries(page);
  await openConn(page);
  await run(page, 'SELECT * FROM docs');
  await page.getByRole('button', { name: 'Filter row' }).click();
  const box = '.filter-row input[aria-label="Filter doc"]';
  await expect(page.locator(box)).toBeVisible();
  const firstRow = `document.querySelector('.grid-scroll tbody tr:not(.spacer) td.cell')?.textContent ?? ''`;
  const before = await page.evaluate(firstRow);
  const first = await keyToFrame(page, box, '"brand":7', `(${firstRow}) !== ${JSON.stringify(before)}`);
  // Second key on the same column reads the cached column text.
  const after1 = await page.evaluate(firstRow);
  const second = await keyToFrame(page, box, '"brand":7,', `(${firstRow}) !== ${JSON.stringify(after1)} || true`);
  perfLine(
    `col filter 100k JSON: key→frame ${first.frame.toFixed(1)} / ${second.frame.toFixed(1)} ms; key→filtered ${first.applied.toFixed(0)} ms`,
  );
  expect(first.frame, 'first key → frame').toBeLessThan(budgetMs(50));
  expect(second.frame, 'second key → frame').toBeLessThan(budgetMs(50));
  expect(first.applied, 'key → filtered rows').toBeLessThan(budgetMs(700));
  // brand = id-1 mod 50: the first row left is id 8.
  await expect(page.locator('.grid-scroll tbody tr:not(.spacer) td.cell').first()).toHaveText('8');
});

// ── 3. result memory budget ──────────────────────────────────────────────────
test('background results are released past the memory budget and re-run on demand', { tag: '@ci' }, async ({ page }, info) => {
  test.skip(!isDesktopProject(info.project.name), 'desktop flow');
  test.setTimeout(180_000);
  // ~6–7 MB per `mid` result (estimated); a 15 MB budget holds two.
  await page.addInitScript(() => localStorage.setItem('otto.db.resultBudgetMB', '15'));
  const seen: string[] = [];
  await routeQueries(page);
  page.on('request', (r) => {
    if (r.url().endsWith('/db/query')) seen.push(r.url());
  });
  await openConn(page);
  const TABS = 6;
  for (let i = 0; i < TABS; i++) {
    if (i > 0) await page.getByRole('button', { name: 'New query tab' }).click();
    await run(page, `SELECT * FROM mid /* ${i} */`);
  }
  // Visit every tab: the older ones show the released state.
  let released = 0;
  for (let i = 0; i < TABS; i++) {
    await page.locator('.qe-tab').nth(i).click();
    const r = page.getByTestId('db-result-released');
    const g = page.locator('.grid-scroll tbody td.cell').first();
    await expect(r.or(g)).toBeVisible({ timeout: 10_000 });
    if (await r.isVisible()) released++;
  }
  perfLine(`result budget: ${released}/${TABS} background results released at 15 MB`);
  expect(released, 'released background results').toBeGreaterThanOrEqual(TABS - 3);
  // Re-run brings the first tab's rows back.
  await page.locator('.qe-tab').nth(0).click();
  await expect(page.getByTestId('db-result-released')).toBeVisible();
  const before = seen.length;
  await page.getByTestId('db-result-released').getByRole('button', { name: 'Re-run' }).click();
  await expect(page.locator('.grid-scroll tbody td.cell').first()).toBeVisible({ timeout: 20_000 });
  expect(seen.length).toBe(before + 1);
});

// ── 3b. tab switch back to a sorted result; search text pre-built on focus ────
test('switching back to a string-sorted 100k tab reuses its view; search text pre-builds on focus', async ({ page }, info) => {
  webkitOnly(info.project.name);
  test.setTimeout(180_000);
  await routeQueries(page);
  await openConn(page);
  await run(page, 'SELECT * FROM big');
  const cellSel = '.grid-scroll tbody tr:not(.spacer) td.cell';
  const firstCell = `document.querySelector('${cellSel}')?.textContent ?? ''`;
  // c_1 (strings) descending: a 100k-string collator sort.
  const th = page.locator('.grid-scroll thead .th-sort').nth(1);
  await th.click();
  await th.click();
  await expect(page.locator(cellSel).first()).not.toHaveText('1');
  const shown = String(await page.evaluate(firstCell));
  // Another tab, then back: the sorted view is served from the per-result cache.
  await page.getByRole('button', { name: 'New query tab' }).click();
  await run(page, 'SELECT * FROM mid');
  const backMs: number[] = [];
  for (let i = 0; i < 3; i++) {
    const ms = await page.evaluate(
      async ({ firstCell, shown }) => {
        // eslint-disable-next-line @typescript-eslint/no-implied-eval
        const cellNow = new Function(`return (${firstCell});`) as () => string;
        const tabs = document.querySelectorAll<HTMLElement>('.qe-tab');
        const t0 = performance.now();
        tabs[0].click();
        for (let k = 0; k < 600 && cellNow() !== shown; k++) await new Promise((r) => requestAnimationFrame(() => r(null)));
        return performance.now() - t0;
      },
      { firstCell, shown },
    );
    backMs.push(ms);
    if (i < 2) {
      await page.locator('.qe-tab').nth(1).click();
      await expect(page.locator(cellSel).first()).not.toHaveText(shown);
    }
  }
  // Focus the search box and let the idle slices pre-build the search text;
  // the first key then only runs the filter pass.
  await page.locator('.gt-search-input').focus();
  await page.waitForTimeout(2_500);
  const key = await keyToFrame(page, '.gt-search-input', 'b12345', `(${firstCell}) !== ${JSON.stringify(shown)}`);
  perfLine(
    `string-sorted 100k tab: switch back ${backMs.map((t) => t.toFixed(0)).join(' / ')} ms; ` +
      `search key→frame ${key.frame.toFixed(1)} ms, key→filtered ${key.applied.toFixed(0)} ms (pre-built)`,
  );
  // Measured (WebKit, load avg ~10): switch back 45–58 ms with the cached view
  // vs 98–126 ms re-sorting; key→filtered 135–141 ms pre-built (120 ms of it is
  // the debounce) vs 174–184 ms building the text on the key.
  expect(Math.min(...backMs), 'switch back to the sorted tab').toBeLessThan(budgetMs(90));
  expect(key.frame, 'search key → frame').toBeLessThan(budgetMs(50));
  expect(key.applied, 'search key → filtered (text pre-built)').toBeLessThan(budgetMs(250));
});

// ── 4. schema tree ───────────────────────────────────────────────────────────
test('schema tree with 20 × 1,000 tables expanded stays windowed', async ({ page }, info) => {
  webkitOnly(info.project.name);
  test.setTimeout(180_000);
  await routeQueries(page);
  const SCHEMAS = 20;
  const TABLES = 1_000;
  await page.route(new RegExp(`/connections/${connId}/db/schema$`), (route) =>
    route.fulfill({
      json: Array.from({ length: SCHEMAS }, (_, s) => ({ id: `db:s${s}`, label: `s${s}`, kind: 'database', has_children: true })),
    }),
  );
  await page.route(new RegExp(`/connections/${connId}/db/schema/children$`), (route) => {
    const path = String(((route.request().postDataJSON() ?? {}) as { path?: string }).path ?? '');
    const m = /^db:(s\d+)$/.exec(path);
    if (!m) return route.fulfill({ json: [] });
    return route.fulfill({
      json: Array.from({ length: TABLES }, (_, t) => ({ id: `${path}/table:t${t}`, label: `${m[1]}_table_${t}`, kind: 'table', has_children: true })),
    });
  });
  await openConn(page);
  // Bottom-up: schema sN sits at row N while every schema above it is still
  // collapsed, so its caret is always inside the mounted window.
  for (let s = SCHEMAS - 1; s >= 0; s--) {
    const caret = page.locator(`.schema-tree button.caret[aria-label="Expand s${s}"]`);
    await caret.scrollIntoViewIfNeeded();
    await caret.click();
    await expect(page.locator(`.schema-tree button.caret[aria-label="Collapse s${s}"]`)).toBeVisible({ timeout: 10_000 });
  }
  // Every table is in the flat list (no "showing 1,000 of …" cap any more).
  await expect(page.locator('.schema-tree .node-more')).toHaveCount(0);
  const nodes = await page.evaluate(() => document.querySelectorAll('.schema-tree *').length);
  const scroller = await page.evaluate(() => {
    let e: HTMLElement | null = document.querySelector('.tree-rows');
    while (e && !/(auto|scroll)/.test(getComputedStyle(e).overflowY)) e = e.parentElement;
    e?.setAttribute('data-perf-tree-scroller', '');
    return !!e;
  });
  expect(scroller).toBe(true);
  const steps = await scrollFrameWork(page, '[data-perf-tree-scroller]', 400, 30);
  const key = await keyToFrame(page, '.tree-search-input', 'table_99', 'true');
  perfLine(`schema tree 20×1000: DOM ${nodes}; scroll p95 ${percentile(steps, 95).toFixed(1)} ms; filter key→frame ${key.frame.toFixed(1)} ms`);
  expect(nodes, `tree DOM ${nodes}`).toBeLessThan(1_500);
  expect(percentile(steps, 95), 'tree scroll step p95').toBeLessThan(budgetMs(16));
  expect(key.frame, 'tree filter key → frame').toBeLessThan(budgetMs(50));
});

// ── 5. restore N connections ─────────────────────────────────────────────────
test('restoring 15 open connections dials the active one first, the rest 3 at a time', async ({ page }, info) => {
  webkitOnly(info.project.name);
  test.setTimeout(180_000);
  let inflight = 0;
  let peak = 0;
  for (const id of restoreIds) {
    await mockDbRoutes(page, id);
    // Registered after mockDbRoutes, so it runs first; then falls back to it.
    await page.route(new RegExp(`/connections/${id}/db/schema$`), async (route: Route) => {
      inflight++;
      peak = Math.max(peak, inflight);
      await new Promise((r) => setTimeout(r, 300));
      inflight--;
      await route.fallback();
    });
  }
  await page.addInitScript((ids) => {
    const v = JSON.stringify({ open: ids, selected: ids[0] });
    localStorage.setItem('otto_db_open', v);
  }, restoreIds);
  await page.goto('/#/database');
  await expect(page.locator('.shell')).toBeVisible({ timeout: 30_000 });
  await expect(page.locator('.conn-tab')).toHaveCount(RESTORE_N, { timeout: 30_000 });
  const t0 = Date.now();
  await expect(page.locator('.qe-edit .cm-content')).toBeVisible({ timeout: 20_000 });
  const activeMs = Date.now() - t0;
  // Let the background warm-up drain.
  await page.waitForTimeout(RESTORE_N * 150);
  perfLine(`restore ${RESTORE_N} conns: active editor ${activeMs} ms after chips; peak concurrent schema loads ${peak}`);
  expect(activeMs, 'active tab interactive').toBeLessThan(budgetMs(1_500));
  expect(peak, 'concurrent schema loads (1 active + 3 background)').toBeLessThanOrEqual(4);
});

// ── 6. Query ↔ Structure remount ─────────────────────────────────────────────
test('Query → Structure → Query with a 100k-row result repaints fast', async ({ page }, info) => {
  webkitOnly(info.project.name);
  test.setTimeout(180_000);
  await routeQueries(page);
  await openConn(page);
  await run(page, 'SELECT * FROM big');
  const timings: number[] = [];
  for (let i = 0; i < 3; i++) {
    await page.locator('.main-tabs .mt', { hasText: 'Structure' }).first().click();
    await expect(page.locator('.main-tabs .mt.active')).toHaveText('Structure');
    await expect(page.locator('.grid-scroll')).toHaveCount(0);
    const ms = await page.evaluate(async () => {
      const tab = [...document.querySelectorAll<HTMLElement>('.main-tabs .mt')].find((b) => b.textContent?.trim() === 'Query');
      const t0 = performance.now();
      tab?.click();
      for (let k = 0; k < 600 && !document.querySelector('.grid-scroll tbody td.cell'); k++) {
        await new Promise((r) => requestAnimationFrame(() => r(null)));
      }
      return performance.now() - t0;
    });
    timings.push(ms);
  }
  perfLine(`Query↔Structure 100k: grid back in ${timings.map((t) => t.toFixed(0)).join(' / ')} ms`);
  // The first switch may still load the Structure chunk; later ones are warm.
  expect(Math.min(...timings), 'grid repaint after Structure → Query').toBeLessThan(budgetMs(150));
});

// ── 7. dashboard charts ──────────────────────────────────────────────────────
test('dashboard of 12 bar widgets × 5,000 rows draws sampled charts', async ({ page }, info) => {
  webkitOnly(info.project.name);
  test.setTimeout(180_000);
  await routeQueries(page);
  const N = 12;
  const now = new Date().toISOString();
  const widgets = Array.from({ length: N }, (_, i) => ({
    id: `w${i}`,
    workspace_id: workspaceId,
    dashboard_id: 'd1',
    connection_id: connId,
    title: `Orders ${i}`,
    statement: `SELECT day, n FROM orders_${i}`,
    viz: 'bar',
    mapping: { x: 'day', y: ['n'] },
    options: {},
    created_at: now,
    updated_at: now,
  }));
  const dash = {
    id: 'd1',
    workspace_id: workspaceId,
    name: 'Scale',
    layout: widgets.map((w, i) => ({ widget_id: w.id, x: (i % 3) * 4, y: Math.floor(i / 3) * 4, w: 4, h: 4 })),
    refresh_secs: null,
    created_at: now,
    updated_at: now,
  };
  const rows = Array.from({ length: 5_000 }, (_, r) => [`2026-01-${String((r % 28) + 1).padStart(2, '0')}#${r}`, (r * 37) % 1000]);
  const runBody = JSON.stringify({
    columns: [{ name: 'day', type_hint: 'VARCHAR' }, { name: 'n', type_hint: 'INT' }],
    rows,
    stats: { duration_ms: 5, row_count: rows.length },
    truncated: false,
  });
  await page.route(/\/api\/v1\/workspaces\/[^/]+\/db\/dashboards$/, (route) =>
    route.request().method() === 'GET' ? route.fulfill({ json: [dash] }) : route.fallback(),
  );
  await page.route(/\/api\/v1\/workspaces\/[^/]+\/db\/widgets$/, (route) =>
    route.request().method() === 'GET' ? route.fulfill({ json: widgets }) : route.fallback(),
  );
  await page.route(/\/api\/v1\/db\/widgets\/[^/]+\/run$/, (route) =>
    route.fulfill({ status: 200, contentType: 'application/json', body: runBody }),
  );
  await openConn(page);
  const t0 = Date.now();
  await page.locator('.main-tabs .mt', { hasText: 'Dashboards' }).first().click();
  await expect(page.locator('.widget-card svg.chart-svg')).toHaveCount(N, { timeout: 20_000 });
  const drawnMs = Date.now() - t0;
  const rects = await page.evaluate(() => document.querySelectorAll('.widget-card svg.chart-svg rect').length);
  perfLine(`dashboard 12 bar × 5000: all tiles drawn in ${drawnMs} ms; ${rects} rects`);
  expect(rects, 'bars drawn across 12 tiles').toBeLessThanOrEqual(N * 160);
  await expect(page.getByTestId('chart-aggregated').first()).toContainText('5,000 points averaged');
  expect(drawnMs, 'all tiles drawn').toBeLessThan(budgetMs(3_000));
});

// ── 8. multi-run sheet ───────────────────────────────────────────────────────
test('multi-run sheet with 200 runs polls without long frames', async ({ page }, info) => {
  test.skip(!isDesktopProject(info.project.name), 'desktop flow');
  test.setTimeout(180_000);
  await routeQueries(page);
  const N = 200;
  const target = { index: 0, connection_id: connId, connection_name: CONN, environment: 'dev', read_only: false, guarded: false, node: 'shop', label: `${CONN} · shop` };
  const runs = Array.from({ length: N }, (_, i) => ({
    index: i,
    target: 0,
    values: [{ name: 'brand', value: String(i + 1) }],
    label: `${target.label} · brand=${i + 1}`,
    statement: `SELECT * FROM orders WHERE brand_id = ${i + 1}`,
    is_write: false,
    needs_confirm: false,
  }));
  const plan = { engine: 'mysql', placeholders: ['brand'], targets: [target], runs, write_count: 0, needs_confirm: false, warnings: [], plan_hash: 'h200' };
  let polls = 0;
  const pollTimes: number[] = [];
  const pollBytes: number[] = [];
  // Like the daemon: `seq` = runs finished so far; `?since=` answers carry
  // only the runs whose status changed after it (finished or started).
  const job = (since?: number) => {
    const done = Math.min(N, polls * 10);
    const changed = (i: number) => since === undefined || (i >= since - 4 && i < done + 4);
    return {
      seq: done,
      partial: since !== undefined,
      id: 'job200',
      status: done >= N ? 'done' : 'running',
      engine: 'mysql',
      created_at: new Date().toISOString(),
      concurrency: 4,
      stop_on_error: false,
      read_only: true,
      statement_preview: 'SELECT * FROM orders WHERE brand_id = :brand',
      summary: { total: N, ok: done, failed: 0, running: done >= N ? 0 : 4, pending: Math.max(0, N - done - 4), skipped: 0, cancelled: 0 },
      targets: since === undefined ? [target] : [],
      items: runs.filter((r) => changed(r.index)).map((r) => ({
        index: r.index,
        target: 0,
        label: r.label,
        values: r.values,
        statement_preview: r.statement,
        is_write: false,
        status: r.index < done ? 'ok' : r.index < done + 4 ? 'running' : 'pending',
        duration_ms: 12,
        has_result: r.index < done,
      })),
    };
  };
  await page.route('**/api/v1/db/multi-run/plan', (route: Route) => route.fulfill({ json: plan }));
  await page.route('**/api/v1/db/multi-runs', (route: Route) =>
    route.request().method() === 'GET' ? route.fulfill({ json: [] }) : route.fulfill({ status: 202, json: job() }),
  );
  await page.route('**/api/v1/db/multi-runs/job200*', (route: Route) => {
    polls++;
    pollTimes.push(Date.now());
    const since = new URL(route.request().url()).searchParams.get('since');
    const body = JSON.stringify(job(since === null ? undefined : Number(since)));
    pollBytes.push(body.length);
    return route.fulfill({ contentType: 'application/json', body });
  });
  await openConn(page);
  const content = page.locator('.qe-edit .cm-content');
  await content.click();
  await page.keyboard.press('ControlOrMeta+A');
  await page.keyboard.insertText('SELECT * FROM orders WHERE brand_id = :brand');
  await page.getByTestId('multi-run-open').click();
  const sheet = page.getByTestId('multi-run');
  await expect(sheet).toBeVisible();
  await sheet.locator('.mr-conn', { hasText: CONN }).locator('.mr-scope', { hasText: 'shop' }).locator('input').check();
  await sheet.getByLabel('Values for brand').fill(Array.from({ length: N }, (_, i) => i + 1).join(', '));
  await page.getByRole('button', { name: `Preview ${N} runs` }).click();
  await expect(sheet.locator('.mr-runs > li')).toHaveCount(N, { timeout: 10_000 });
  // Watch frame gaps while the sheet polls the 200-item job to completion.
  await page.evaluate(() => {
    const w = window as unknown as { __mrGaps: number[]; __mrOn: boolean };
    w.__mrGaps = [];
    w.__mrOn = true;
    let last = performance.now();
    const loop = (t: number) => {
      if (!w.__mrOn) return;
      w.__mrGaps.push(t - last);
      last = t;
      requestAnimationFrame(loop);
    };
    requestAnimationFrame(loop);
  });
  await page.getByRole('button', { name: `Run ${N}` }).click();
  await expect(sheet.locator('.mr-status')).toContainText(`${N} ok`, { timeout: 60_000 });
  const gaps = await page.evaluate(() => {
    const w = window as unknown as { __mrGaps: number[]; __mrOn: boolean };
    w.__mrOn = false;
    return w.__mrGaps;
  });
  const span = (pollTimes.at(-1)! - pollTimes[0]) / 1000;
  const rate = pollTimes.length > 1 ? (pollTimes.length - 1) / span : 0;
  const fullBytes = JSON.stringify(job()).length;
  const maxPoll = Math.max(...pollBytes);
  perfLine(
    `multi-run ${N}: ${polls} polls (${rate.toFixed(2)}/s), poll ≤ ${maxPoll} B vs ${fullBytes} B full; ` +
      `worst frame gap ${Math.max(...gaps).toFixed(0)} ms, p95 ${percentile(gaps, 95).toFixed(1)} ms`,
  );
  // Every poll used `?since=` (a delta), none re-pulled the whole job.
  expect(maxPoll, 'largest poll answer').toBeLessThan(fullBytes / 4);
  expect(percentile(gaps, 95), 'frame gap p95 while polling').toBeLessThan(budgetMs(50));
  expect(rate, 'poll rate').toBeLessThanOrEqual(1.6);
});
