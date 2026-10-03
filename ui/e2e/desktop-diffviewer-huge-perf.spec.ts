import { test, expect, type Locator, type Page } from '@playwright/test';
import { execFileSync } from 'node:child_process';
import { mkdtempSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { apiCtx, seedWorkspace } from './seed';

// ─────────────────────────────────────────────────────────────────────────────
// Performance gate for DiffViewer on ~100k-line diffs (desktop-browser only).
//
// The regressions this guards against were all "a guard that silently doesn't
// run": collapse state filled in AFTER a first render of every row, PR mode
// that never windowed, "Expand all" rendering every line, per-line hljs on the
// main thread. So the assertions are about what actually reaches the DOM and
// the main thread, not about timings of a fast path:
//   • DOM rows stay bounded (O(viewport)) before and after "Expand all";
//   • no long task > 200 ms while opening, expanding and scrolling;
//   • a summary diff fetches hunks only for files near the viewport;
//   • a PR comment deep inside the diff is still reachable (never capped away).
// ─────────────────────────────────────────────────────────────────────────────

test.use({ serviceWorkers: 'block' });
// Desktop Chromium only: the long-task observer is Chromium's, and the mobile
// projects re-run desktop-* specs at phone widths.
test.skip(({ isMobile, browserName }) => isMobile || browserName !== 'chromium', 'desktop Chromium perf gate');

const FILES = 1000;
const LINES = 100; // per file → 100k changed lines
const ROW_BUDGET = 700; // generous: ~2 viewports of rows + overscan at 18–20 px

let repoId = '';
let workspaceId = '';
let wipDir = '';

function hunkFor(i: number) {
  const lines = Array.from({ length: LINES }, (_, j) => ({
    origin: j % 2 === 0 ? 'del' : 'add',
    content: `const value_${i}_${j} = computeSomething(${j}, "a moderately long string literal to widen the row");`,
    old_line: j % 2 === 0 ? j + 1 : null,
    new_line: j % 2 === 0 ? null : j + 1,
  }));
  return { header: `@@ -1,${LINES / 2} +1,${LINES / 2} @@`, lines };
}
function filePath(i: number): string {
  return `src/module_${Math.floor(i / 50)}/file_${i}.ts`;
}
function fullFile(i: number) {
  return {
    path: filePath(i),
    old_path: null,
    is_binary: false,
    hunks: [hunkFor(i)],
    added: LINES / 2,
    deleted: LINES / 2,
    status: 'modified',
  };
}
function summaryFile(i: number) {
  return { ...fullFile(i), hunks: [], hunks_omitted: true };
}

test.beforeAll(async () => {
  const { ctx, base } = await apiCtx();
  workspaceId = await seedWorkspace(ctx, base);
  const dir = mkdtempSync(join(tmpdir(), 'otto-e2e-huge-diff-'));
  const git = (...a: string[]) => execFileSync('git', ['-C', dir, ...a], { stdio: 'ignore' });
  git('init', '-q', '-b', 'main');
  git('config', 'user.email', 'e2e@otto.local');
  git('config', 'user.name', 'E2E');
  git('config', 'commit.gpgsign', 'false');
  // 20k-line file, every line rewritten in the worktree → one 40k-line diff.
  writeFileSync(join(dir, 'big.ts'), Array.from({ length: 20_000 }, (_, i) => `export const a${i} = ${i};\n`).join(''));
  git('add', '.');
  git('commit', '-qm', 'init');
  writeFileSync(join(dir, 'big.ts'), Array.from({ length: 20_000 }, (_, i) => `export const b${i} = ${i * 2};\n`).join(''));
  git('remote', 'add', 'origin', 'https://github.com/otto-test/huge-diff.git');
  const r = await ctx.post(`${base}/api/v1/workspaces/${workspaceId}/repos`, { data: { path: dir, name: 'huge-diff' } });
  expect(r.ok()).toBeTruthy();
  repoId = (await r.json()).id;
  wipDir = dir;
  await ctx.dispose();
});

test.beforeEach(async ({ page }) => {
  await page.addInitScript((id) => {
    localStorage.setItem('otto_workspace', id);
    localStorage.setItem('otto_rail_expanded', '0');
    localStorage.setItem('otto_git_auto_fetch', JSON.stringify({ enabled: false }));
    // Long-task log (Chromium): every main-thread block over 50 ms.
    const w = window as unknown as { __lt: number[] };
    w.__lt = [];
    try {
      new PerformanceObserver((list) => {
        for (const e of list.getEntries()) w.__lt.push(e.duration);
      }).observe({ type: 'longtask', buffered: true });
    } catch {
      /* not supported */
    }
  }, workspaceId);
});

async function resetLongTasks(page: Page): Promise<void> {
  await page.evaluate(() => ((window as unknown as { __lt: number[] }).__lt = []));
}
async function maxLongTask(page: Page): Promise<number> {
  return page.evaluate(() => Math.max(0, ...(window as unknown as { __lt: number[] }).__lt));
}
async function domRows(page: Page): Promise<number> {
  return page.evaluate(() => document.querySelectorAll('.diff-body .vrow, .diff-body tr').length);
}
/** Scroll the diff's real scroller down one viewport at a time until
 *  `target` is mounted (the diff is windowed: a row far below the view is not
 *  in the DOM, so `scrollIntoViewIfNeeded` alone can never find it). */
async function scrollUntilMounted(page: Page, target: Locator, max = 60): Promise<void> {
  for (let k = 0; k < max; k++) {
    if ((await target.count()) > 0) return;
    await scrollViewports(page, 1);
  }
  throw new Error(`not mounted after scrolling ${max} viewports`);
}

/** Scroll the diff's real scroller through `n` viewports. */
async function scrollViewports(page: Page, n: number): Promise<void> {
  for (let k = 0; k < n; k++) {
    await page.evaluate(() => {
      const body = document.querySelector('.diff-body') as HTMLElement;
      let p = body.parentElement;
      while (p && !(/(auto|scroll)/.test(getComputedStyle(p).overflowY) && p.scrollHeight > p.clientHeight + 1)) {
        p = p.parentElement;
      }
      const sc = p ?? document.scrollingElement!;
      sc.scrollTop += sc.clientHeight;
    });
    await page.evaluate(() => new Promise((r) => requestAnimationFrame(() => requestAnimationFrame(r))));
  }
}

async function prRoutes(page: Page, mode: 'summary' | 'legacy', perFile: string[]): Promise<void> {
  const comment = {
    id: 'deep-comment',
    author: 'reviewer',
    body: 'Deep comment far down the diff',
    path: filePath(FILES - 3),
    line: 60,
    created_at: '2026-09-25T08:00:00Z',
    replies: [],
    resolved: false,
  };
  await page.route('**/api/v1/repos/*/prs/*', async (route) => {
    const path = new URL(route.request().url()).pathname;
    if (!/\/prs\/\d+$/.test(path)) return route.fallback();
    await route.fulfill({
      json: {
        number: 1,
        title: 'Huge PR',
        state: 'open',
        author: 'perf',
        source_branch: 'feature/huge',
        target_branch: 'main',
        url: 'https://github.com/otto-test/huge-diff/pull/1',
        updated_at: '2026-09-25T08:00:00Z',
        draft: false,
        description_md: 'Huge change',
        approved_by: [],
        reviewers: [],
        mergeable: true,
        comments: [comment],
      },
    });
  });
  await page.route('**/api/v1/repos/*/prs/*/diff*', async (route) => {
    const url = new URL(route.request().url());
    const p = url.searchParams.get('path');
    if (p !== null) {
      perFile.push(p);
      const i = Number(/file_(\d+)\.ts$/.exec(p)?.[1] ?? -1);
      return route.fulfill({ json: { files: i >= 0 ? [fullFile(i)] : [] } });
    }
    const files = Array.from({ length: FILES }, (_, i) => (mode === 'summary' ? summaryFile(i) : fullFile(i)));
    await route.fulfill({ json: { files, total_added: (FILES * LINES) / 2, total_deleted: (FILES * LINES) / 2 } });
  });
  await page.route('**/api/v1/repos/*/prs/*/commits', (r) => r.fulfill({ json: [] }));
  await page.route('**/api/v1/repos/*/prs/*/reviews', (r) => r.fulfill({ json: [] }));
}

for (const mode of ['summary', 'legacy'] as const) {
  test(`PR diff (${mode}, ${FILES} files × ${LINES} lines): bounded DOM, no long tasks, bounded Expand all`, async ({ page }) => {
    test.setTimeout(120_000);
    const perFile: string[] = [];
    // G4: the variable-height measurer must not re-lay out inside its own
    // ResizeObserver callback ("ResizeObserver loop completed with
    // undelivered notifications" was logged hundreds of times a second).
    let resizeLoops = 0;
    const loopHit = (t: string): void => {
      if (/ResizeObserver loop/i.test(t)) resizeLoops++;
    };
    page.on('console', (m) => loopHit(m.text()));
    page.on('pageerror', (e) => loopHit(e.message));
    await prRoutes(page, mode, perFile);
    await page.goto(`/#/git/${repoId}/pr/1`);
    await expect(page.getByRole('tab', { name: 'Files', exact: true })).toBeVisible({ timeout: 15_000 });
    await resetLongTasks(page);

    const t0 = Date.now();
    await page.getByRole('tab', { name: 'Files', exact: true }).click();
    await expect(page.locator('.dfile-head').first()).toBeVisible();
    const openMs = Date.now() - t0;
    // > 40 files ⇒ everything starts collapsed: headers only, no line rows.
    expect(await domRows(page)).toBe(0);
    const headers = await page.locator('.dfile-head').count();
    expect(headers, 'file headers are windowed too').toBeLessThan(200);
    expect(await maxLongTask(page), `open took ${openMs} ms`).toBeLessThan(mode === 'legacy' ? 400 : 200);

    // Expand all: still O(viewport).
    await resetLongTasks(page);
    await page.getByRole('button', { name: 'Expand all' }).click();
    await expect(page.locator('.diff-body .vrow').first()).toBeVisible({ timeout: 15_000 });
    await page.waitForTimeout(300);
    const afterExpand = await domRows(page);
    expect(afterExpand, 'rows after Expand all').toBeLessThan(ROW_BUDGET);
    expect(await maxLongTask(page)).toBeLessThan(200);
    if (mode === 'summary') {
      // Only the files that reached the window were fetched (4 in flight max).
      expect(perFile.length, 'per-file fetches after Expand all').toBeLessThan(40);
    }

    // Scroll 20 viewports: DOM stays bounded, main thread stays responsive.
    await resetLongTasks(page);
    await scrollViewports(page, 20);
    await page.waitForTimeout(300);
    expect(await domRows(page), 'rows after scrolling').toBeLessThan(ROW_BUDGET);
    expect(await maxLongTask(page)).toBeLessThan(200);

    // Split mode over the same expanded diff.
    await resetLongTasks(page);
    await page.getByRole('button', { name: 'Side by side' }).click();
    await expect(page.locator('.diff-body .split-vrow').first()).toBeVisible();
    expect(await domRows(page)).toBeLessThan(ROW_BUDGET);
    expect(await maxLongTask(page)).toBeLessThan(200);
    await page.getByRole('button', { name: 'Unified' }).click();

    // The navigator is windowed too (GIT2-12): 1k files never mount 1k nav rows.
    expect(await page.locator('.diff-nav .nav-file').count(), 'nav rows mounted').toBeLessThan(120);
    // Jump to a file near the end from the navigator (filter to reach it — its
    // row is outside the nav window): its comment renders.
    await page.locator('.nav-search').fill(`file_${FILES - 3}.ts`);
    await page.locator('.nav-file', { hasText: `file_${FILES - 3}.ts` }).click();
    await expect(page.getByText('Deep comment far down the diff')).toBeVisible({ timeout: 15_000 });
    expect(await domRows(page)).toBeLessThan(ROW_BUDGET);
    expect(resizeLoops, 'no "ResizeObserver loop" errors').toBe(0);
  });
}

test('WIP diff of a 40k-line rewrite: large-file gate, windowed rows, capped hunk', async ({ page }) => {
  test.setTimeout(120_000);
  void wipDir;
  await page.goto(`/#/git/${repoId}/graph`);
  await expect(page.locator('.rv-tabs')).toBeVisible({ timeout: 15_000 });
  const wipRow = page.locator('.wip-row');
  await expect(wipRow).toBeVisible({ timeout: 15_000 });
  await wipRow.click();
  const panel = page.locator('.wip-panel');
  await panel.locator('.wp-name', { hasText: 'big.ts' }).first().click();
  await expect(panel.locator('.wp-diff')).toBeVisible();
  await expect(panel.locator('.dfile-head')).toBeVisible({ timeout: 15_000 });

  await resetLongTasks(page);
  // Capped server: "Large file · Load anyway"; uncapped: the file starts
  // collapsed (>400 lines) — either way nothing renders until asked.
  expect(await domRows(page)).toBe(0);
  const loadAnyway = panel.getByRole('button', { name: 'Load anyway' });
  // A >400-line file starts collapsed — expand it first; a capped server then
  // shows "Load anyway", an uncapped one the (windowed) rows.
  if (!(await loadAnyway.isVisible())) await panel.locator('.dfile-head').click();
  await expect(loadAnyway.or(panel.locator('.diff-body .vrow').first())).toBeVisible({ timeout: 15_000 });
  if (await loadAnyway.isVisible()) await loadAnyway.click();
  await expect(panel.locator('.diff-body .vrow').first()).toBeVisible({ timeout: 30_000 });
  await page.waitForTimeout(300);
  expect(await domRows(page)).toBeLessThan(ROW_BUDGET);

  // The hunk is capped in the row model (500 lines, then a "Show N more
  // lines" row). That row is windowed like any other: scroll down to it.
  const more = panel.getByRole('button', { name: /Show [\d,]+ more lines/ });
  await scrollUntilMounted(page, more);
  await more.scrollIntoViewIfNeeded();
  await resetLongTasks(page);
  await more.click();
  await page.waitForTimeout(300);
  expect(await domRows(page)).toBeLessThan(ROW_BUDGET);
  await scrollViewports(page, 20);
  await page.waitForTimeout(300);
  expect(await domRows(page)).toBeLessThan(ROW_BUDGET);
  expect(await maxLongTask(page)).toBeLessThan(200);
});

// G7: a minified bundle — one 1 MB line. The DOM only gets the first 10k
// chars plus an "… expand line" button; layout of the whole text node used
// to be one long task.
test('PR diff with a 1 MB single-line (minified) file: line cut, expand on demand, no long task', async ({ page }) => {
  test.setTimeout(60_000);
  const big = 'var a=1;'.repeat(131_072); // 1 MiB
  const file = {
    path: 'dist/bundle.min.js',
    old_path: null,
    is_binary: false,
    hunks: [{ header: '@@ -0,0 +1,1 @@', lines: [{ origin: 'add', content: big, old_line: null, new_line: 1 }] }],
    added: 1,
    deleted: 0,
    status: 'added',
  };
  await page.route('**/api/v1/repos/*/prs/*', async (route) => {
    const path = new URL(route.request().url()).pathname;
    if (!/\/prs\/\d+$/.test(path)) return route.fallback();
    await route.fulfill({
      json: {
        number: 2, title: 'Minified', state: 'open', author: 'perf', source_branch: 'feature/min',
        target_branch: 'main', url: 'https://github.com/otto-test/huge-diff/pull/2',
        updated_at: '2026-09-25T08:00:00Z', draft: false, description_md: '', approved_by: [],
        reviewers: [], mergeable: true, comments: [],
      },
    });
  });
  await page.route('**/api/v1/repos/*/prs/*/diff*', (r) => r.fulfill({ json: { files: [file], total_added: 1, total_deleted: 0 } }));
  await page.route('**/api/v1/repos/*/prs/*/commits', (r) => r.fulfill({ json: [] }));
  await page.route('**/api/v1/repos/*/prs/*/reviews', (r) => r.fulfill({ json: [] }));
  await page.goto(`/#/git/${repoId}/pr/2`);
  await expect(page.getByRole('tab', { name: 'Files', exact: true })).toBeVisible({ timeout: 15_000 });
  await resetLongTasks(page);
  await page.getByRole('tab', { name: 'Files', exact: true }).click();
  const expand = page.getByRole('button', { name: /Expand line \(1\.0 MB\)/ });
  await expect(expand).toBeVisible({ timeout: 15_000 });
  await page.waitForTimeout(300);
  const textLen = await page.evaluate(() => (document.querySelector('.diff-body .dline .code') as HTMLElement).textContent!.length);
  expect(textLen, 'only the cut prefix reaches the DOM').toBeLessThan(10_100);
  expect(await maxLongTask(page)).toBeLessThan(200);
  // Expanding is explicit and still renders the whole line.
  await expand.click();
  await expect(expand).toHaveCount(0);
  const full = await page.evaluate(() => (document.querySelector('.diff-body .dline .code') as HTMLElement).textContent!.length);
  expect(full).toBeGreaterThanOrEqual(big.length);
});
