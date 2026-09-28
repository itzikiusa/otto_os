import { test, expect, type Page } from '@playwright/test';
import { apiCtx, seedWorkspace } from './seed';
import { mockDbRoutes, seedMockDbConnection } from './db-mock';

// ─────────────────────────────────────────────────────────────────────────────
// DB Explorer — results grid at scale (perf regression gate, GAPS §3 / I1).
//
// Scenario sizes:
//   • `SELECT * FROM big`  → 100,000 rows × 30 columns (≈ 20 MB of JSON, under
//     the daemon's 32 MB response budget);
//   • `SELECT * FROM fat`  → 40 rows whose `doc` column is a ~50 KB document.
// Metrics and budgets (WebKit ≈ Tauri's WKWebView; mocked, Docker-free):
//   • mounted DOM under the grid                 < 4,000 nodes
//   • scroll step (300 px) work, p95             < 12 ms   (was 13–17 ms: per-cell Icon + closures)
//   • jump to the middle                         < 35 ms   (was 45–54 ms)
//   • sort click → sorted first row painted      < 250 ms  (was 1.49 s)
//   • search: key → next frame                   < 50 ms   (was 0.72–0.90 s)
//     and key → filtered rows painted            < 600 ms  (filter applies after a 120 ms pause above 20k rows)
//   • 40 × 50 KB documents scrolled in, per step < 16 ms   (was 38–54 ms: whole-doc stringify per cell)
// Work is timed WITHOUT waiting for vsync: set scrollTop, fire the scroll
// handler, let Svelte flush, force layout — so the numbers are script +
// style + layout per step, comparable across refresh rates.
// ─────────────────────────────────────────────────────────────────────────────

test.use({ browserName: 'webkit', serviceWorkers: 'block' });

let workspaceId = '';
let connId = '';
const CONN = `mock-results-${Math.random().toString(36).slice(2, 8)}`;

const BIG_ROWS = 100_000;
const BIG_COLS = 30;
const FAT_ROWS = 40;

test.beforeAll(async () => {
  test.setTimeout(120_000);
  const { ctx, base } = await apiCtx();
  workspaceId = await seedWorkspace(ctx, base);
  connId = await seedMockDbConnection(ctx, base, workspaceId, CONN);
  await ctx.dispose().catch(() => {});
});

test.beforeEach(async ({ page }, testInfo) => {
  test.skip(testInfo.project.name !== 'desktop-browser', 'desktop-browser only');
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.addInitScript((wsId) => {
    localStorage.setItem('otto_workspace', wsId as string);
    localStorage.setItem('otto_rail_expanded', '0');
  }, workspaceId);
});

/** 100k × 30 — ints, short strings, decimals, a nullable column. Built once. */
let bigBody: string | null = null;
function bigResult(): string {
  if (bigBody) return bigBody;
  const columns = Array.from({ length: BIG_COLS }, (_, c) => ({
    name: c === 0 ? 'id' : `col_${c}`,
    type_hint: c === 0 ? 'BIGINT' : c % 3 === 0 ? 'DECIMAL(10,2)' : c % 3 === 1 ? 'VARCHAR(40)' : 'INT',
  }));
  const rows: unknown[][] = new Array(BIG_ROWS);
  for (let r = 0; r < BIG_ROWS; r++) {
    const row: unknown[] = new Array(BIG_COLS);
    row[0] = r + 1;
    for (let c = 1; c < BIG_COLS; c++) {
      if (c === 7 && r % 5 === 0) row[c] = null;
      else if (c % 3 === 0) row[c] = ((r * c) % 9000 / 7).toFixed(2);
      else if (c % 3 === 1) row[c] = `v${(r * 31 + c) % 50_000}`;
      else row[c] = (r * 13 + c) % 997;
    }
    rows[r] = row;
  }
  bigBody = JSON.stringify({
    columns,
    rows,
    stats: { duration_ms: 40, row_count: BIG_ROWS },
    truncated: false,
  });
  return bigBody;
}

/** 40 rows, each with a ~50 KB nested `doc`. */
function fatResult(): string {
  const doc = (d: number) => ({
    meta: { brand_id: 1000 + (d % 2), revision: d },
    structure: Array.from({ length: 300 }, (_, i) => ({
      name: `cat-${i}`,
      category_id: `c${i}`,
      games: ['g1', 'g2', 'g3'],
      spec: { provider: `p${i}`, live: i % 2 === 0 },
    })),
  });
  return JSON.stringify({
    columns: [
      { name: 'id', type_hint: 'INT' },
      { name: 'doc', type_hint: 'JSON' },
    ],
    rows: Array.from({ length: FAT_ROWS }, (_, d) => [d + 1, doc(d)]),
    stats: { duration_ms: 12, row_count: FAT_ROWS },
    truncated: false,
  });
}

async function openAndRun(page: Page, statement: string): Promise<void> {
  await mockDbRoutes(page, connId);
  // Registered after mockDbRoutes, so it wins for /query.
  await page.route(new RegExp(`/connections/${connId}/db/query$`), (route) => {
    const body = (route.request().postDataJSON() ?? {}) as { statement?: string };
    const stmt = String(body.statement ?? '');
    const payload = /\bfat\b/i.test(stmt) ? fatResult() : bigResult();
    return route.fulfill({ status: 200, contentType: 'application/json', body: payload });
  });
  await page.goto('/#/database');
  await expect(page.locator('.shell')).toBeVisible({ timeout: 30_000 });
  const c = page.locator('.conn-list .conn-name', { hasText: CONN });
  await expect(c.first()).toBeVisible({ timeout: 30_000 });
  await c.first().click();
  const content = page.locator('.qe-edit .cm-content');
  await expect(content).toBeVisible({ timeout: 20_000 });
  await content.click();
  await page.keyboard.press('ControlOrMeta+A');
  await page.keyboard.insertText(statement);
  await page.waitForTimeout(250);
  await page.locator('.btn.small.primary', { hasText: 'Run' }).first().click();
  await expect(page.locator('.grid-scroll tbody td.cell').first()).toBeVisible({ timeout: 60_000 });
}

/** Time one programmatic scroll: script + style + layout, no vsync wait.
 *  The sample ends on a microtask, not a MessageChannel task: each step
 *  starts in a rAF callback, so a posted task always ran after that frame's
 *  whole style/layout/PAINT and every sample counted a paint. */
async function scrollSteps(page: Page, dy: number, steps: number): Promise<number[]> {
  return page.evaluate(
    async ({ dy, steps }) => {
      const el = document.querySelector('.grid-scroll') as HTMLElement;
      // The scroll handler queued Svelte's flush microtask first, so it (and
      // every effect it runs) has finished when this one resolves.
      const flushed = () => new Promise<void>((r) => queueMicrotask(r));
      const out: number[] = [];
      for (let i = 0; i < steps; i++) {
        const t0 = performance.now();
        el.scrollTop += dy;
        el.dispatchEvent(new Event('scroll'));
        await flushed();
        void (document.body as HTMLElement).offsetHeight; // force style + layout
        out.push(performance.now() - t0);
        await new Promise((r) => requestAnimationFrame(() => r(null)));
      }
      return out;
    },
    { dy, steps },
  );
}

const pct = (xs: number[], p: number): number => {
  const s = [...xs].sort((a, b) => a - b);
  return s[Math.min(s.length - 1, Math.floor(p * s.length))] ?? 0;
};

test('100k × 30 result: bounded DOM, fast scroll / jump / sort / search', async ({ page }) => {
  test.setTimeout(180_000);
  await openAndRun(page, 'SELECT * FROM big');

  // 1) Mounted DOM stays bounded (windowed rows, no per-cell Icon component).
  const nodes = await page.evaluate(() => document.querySelectorAll('.grid-scroll *').length);
  expect(nodes, `grid DOM nodes ${nodes}`).toBeLessThan(4_000);

  // 2) Scroll steps.
  await scrollSteps(page, 300, 5); // warm-up
  const steps = await scrollSteps(page, 300, 40);
  const stepP95 = pct(steps, 0.95);

  // 3) Jump to the middle.
  const jump = await page.evaluate(async () => {
    const el = document.querySelector('.grid-scroll') as HTMLElement;
    const t0 = performance.now();
    el.scrollTop = el.scrollHeight / 2;
    el.dispatchEvent(new Event('scroll'));
    await new Promise<void>((r) => queueMicrotask(r)); // Svelte's flush ran first
    void (document.body as HTMLElement).offsetHeight;
    return performance.now() - t0;
  });

  // 4) Sort click → the first row shows the other end of the order.
  await page.evaluate(() => ((document.querySelector('.grid-scroll') as HTMLElement).scrollTop = 0));
  const sortMs = await page.evaluate(async () => {
    const th = document.querySelectorAll('.grid-scroll thead .th-sort')[0] as HTMLElement;
    const firstCell = () => document.querySelector('.grid-scroll tbody tr:not(.spacer) td.cell')?.textContent ?? '';
    const before = firstCell();
    const t0 = performance.now();
    th.click(); // asc
    th.click(); // desc — the first row now changes
    for (let i = 0; i < 400 && firstCell() === before; i++) {
      await new Promise((r) => requestAnimationFrame(() => r(null)));
    }
    return performance.now() - t0;
  });

  // 5) Search: the key paints at once; the filter applies after a short pause.
  const search = page.locator('.gt-search-input');
  await search.click();
  const searchMs = await page.evaluate(async () => {
    const input = document.querySelector('.gt-search-input') as HTMLInputElement;
    const firstRow = () => document.querySelector('.grid-scroll tbody tr:not(.spacer)')?.textContent ?? '';
    const before = firstRow();
    const t0 = performance.now();
    input.value = 'v12345';
    input.dispatchEvent(new Event('input', { bubbles: true }));
    await new Promise((r) => requestAnimationFrame(() => setTimeout(r, 0)));
    const keyFrame = performance.now() - t0;
    // The unfiltered first row (id 100000 after the desc sort) has no `v12345`.
    for (let i = 0; i < 600 && firstRow() === before; i++) {
      await new Promise((r) => requestAnimationFrame(() => r(null)));
    }
    return { keyFrame, applied: performance.now() - t0 };
  });
  await expect(page.locator('.grid-scroll tbody tr:not(.spacer)').first()).toBeVisible();

  const perfLine =
      `100k×30: DOM ${nodes} nodes; scroll step p50 ${pct(steps, 0.5).toFixed(1)} / p95 ${stepP95.toFixed(1)} ms; ` +
      `jump ${jump.toFixed(1)} ms; sort ${sortMs.toFixed(0)} ms; search key→frame ${searchMs.keyFrame.toFixed(1)} ms, ` +
      `key→filtered ${searchMs.applied.toFixed(0)} ms`;
  console.log(`[perf] ${perfLine}`);
  test.info().annotations.push({ type: 'perf', description: perfLine });
  expect(stepP95, `scroll step p50 ${pct(steps, 0.5).toFixed(1)} / p95 ${stepP95.toFixed(1)} ms`).toBeLessThan(12);
  expect(jump, `jump ${jump.toFixed(1)} ms`).toBeLessThan(35);
  expect(sortMs, `sort ${sortMs.toFixed(0)} ms`).toBeLessThan(250);
  expect(searchMs.keyFrame, `search key→frame ${searchMs.keyFrame.toFixed(1)} ms`).toBeLessThan(50);
  expect(searchMs.applied, `search key→filtered ${searchMs.applied.toFixed(0)} ms`).toBeLessThan(600);
});

test('40 × 50 KB documents: complex cells preview without serializing whole docs', async ({ page }) => {
  test.setTimeout(120_000);
  await openAndRun(page, 'SELECT * FROM fat');
  await expect(page.locator('.grid-scroll td.cell.json').first()).toBeVisible();
  // Every rendered preview is clipped (≤ 512 chars + the ellipsis).
  const maxLen = await page.evaluate(() =>
    Math.max(...[...document.querySelectorAll('.grid-scroll td.cell.json')].map((td) => (td.firstChild?.textContent ?? '').length)),
  );
  expect(maxLen).toBeLessThanOrEqual(513);
  // Scroll the documents in and out: each step re-renders newly visible cells.
  const steps = [...(await scrollSteps(page, 120, 12)), ...(await scrollSteps(page, -120, 12))];
  const worst = Math.max(...steps);
  const fatLine = `fat docs: scroll-in step p50 ${pct(steps, 0.5).toFixed(1)} ms, max ${worst.toFixed(1)} ms`;
  console.log(`[perf] ${fatLine}`);
  test.info().annotations.push({ type: 'perf', description: fatLine });
  expect(pct(steps, 0.95), `fat-doc scroll step p95 ${pct(steps, 0.95).toFixed(1)} ms`).toBeLessThan(16);
  // The expand glyph is CSS, not an <svg> per cell.
  expect(await page.locator('.grid-scroll .cell-expand svg').count()).toBe(0);
});
