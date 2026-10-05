import { test, expect, type Page, type Request } from '@playwright/test';
import { execFileSync } from 'node:child_process';
import { mkdtempSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { apiCtx, seedWorkspace } from './seed';
import { openPage } from './helpers';
import { heapAfterGC } from './perf';

// ─────────────────────────────────────────────────────────────────────────────
// PERF GATE — clicking a branch whose tip commit carries ~100k changed lines
// (desktop-browser only).
//
// The regression this locks down: a single click on such a branch fetched the
// tip's FULL diff (~12 MB JSON) and rendered every line as a table row
// (~500k DOM nodes) — a 2–3 s frozen main thread, then ~0.6 s per relayout.
// A double-click (checkout) paid all of that too, then re-read 10k commits.
// Separately, every graph refresh asked the daemon for one more page of
// history than the last (`limit` 10k → 20k → 30k…).
//
// Fixture (built with `git fast-import`, ~2 s): `main` = 10,050 tiny commits
// (more than one 10k history page, so the refresh-growth bug can show),
// `huge` = main + ONE commit adding 400 files × 250 lines (100k lines), and
// `stale` = a branch at main~10040 — its tip lies BEYOND the first history
// page, like any old branch in a big repo. An unchanged ref like that must
// never count as "history moved" (it used to make every checkout, auto-fetch
// and remount re-read `log --all -n 10000`).
//
// Serial: the double-click checks `huge` out and the refresh test commits on
// top of it, so the tests share one on-disk repo in order.
// ─────────────────────────────────────────────────────────────────────────────

test.describe.configure({ mode: 'serial' });
// The mobile projects have no testMatch, so they'd run this too (and rebuild
// the 10k-commit fixture per project); `longtask` is also Chromium-only.
test.skip(({ isMobile, browserName }) => isMobile || browserName !== 'chromium', 'desktop Chromium perf gate');

const REPO_NAME = 'e2e-huge-branch-perf';
const HUGE_FILES = 400;
const HUGE_LINES = 250;
const MAIN_COMMITS = 10_050;
let repoDir = '';

function git(dir: string, ...args: string[]): string {
  return execFileSync('git', ['-C', dir, ...args], { encoding: 'utf8', maxBuffer: 64 << 20 }).trim();
}

/** A fast-import stream: MAIN_COMMITS small commits on main, then `huge`. */
function fastImportStream(): string {
  const out: string[] = [];
  const data = (s: string) => `data ${Buffer.byteLength(s)}\n${s}\n`;
  const who = 'committer E2E <e2e@otto.local>';
  let t = 1_700_000_000;
  for (let i = 1; i <= MAIN_COMMITS; i++) {
    out.push(`commit refs/heads/main\nmark :${i}\n${who} ${t++} +0000\n${data(`main ${i}`)}`);
    if (i > 1) out.push(`from :${i - 1}\n`);
    out.push(`M 100644 inline log.txt\n${data(`${i}\n`)}\n`);
  }
  out.push(`commit refs/heads/huge\nmark :${MAIN_COMMITS + 1}\n${who} ${t++} +0000\n${data('huge tip commit')}`);
  out.push(`from :${MAIN_COMMITS}\n`);
  for (let f = 0; f < HUGE_FILES; f++) {
    const lines: string[] = [];
    for (let l = 0; l < HUGE_LINES; l++) lines.push(`fn f_${f}_${l}() -> u64 { ${(f * 7919 + l * 104_729) % 1_000_003} }`);
    out.push(`M 100644 inline src/mod${String(f % 20).padStart(2, '0')}/file${String(f).padStart(4, '0')}.rs\n${data(lines.join('\n') + '\n')}`);
  }
  out.push('\n');
  return out.join('');
}

test.beforeAll(async ({}, testInfo) => {
  test.skip(testInfo.project.name !== 'desktop-browser', 'desktop-browser only — no fixture build elsewhere');
  test.setTimeout(120_000);
  const { ctx, base } = await apiCtx();
  const wsId = await seedWorkspace(ctx, base);

  const root = mkdtempSync(join(tmpdir(), 'otto-e2e-huge-'));
  repoDir = join(root, 'work');
  const origin = join(root, 'origin.git');
  execFileSync('git', ['init', '-q', '-b', 'main', repoDir]);
  git(repoDir, 'config', 'user.email', 'e2e@otto.local');
  git(repoDir, 'config', 'user.name', 'E2E');
  git(repoDir, 'config', 'commit.gpgsign', 'false');
  execFileSync('git', ['-C', repoDir, 'fast-import', '--quiet'], { input: fastImportStream(), maxBuffer: 64 << 20 });
  git(repoDir, 'checkout', '-q', '-f', 'main');
  git(repoDir, 'branch', 'stale', `main~${MAIN_COMMITS - 10}`);
  // A local bare origin so the toolbar's Fetch (the refresh trigger) works.
  execFileSync('git', ['init', '-q', '--bare', '-b', 'main', origin]);
  git(repoDir, 'remote', 'add', 'origin', origin);
  git(repoDir, 'push', '-q', 'origin', 'main');

  const r = await ctx.post(`${base}/api/v1/workspaces/${wsId}/repos`, {
    data: { path: repoDir, name: REPO_NAME },
  });
  if (!r.ok()) throw new Error(`repo seed failed: ${r.status()} ${await r.text()}`);
  await ctx.dispose();
});

async function openRepo(page: Page): Promise<void> {
  await openPage(page, 'git');
  const existingTab = page.locator('.git-tab-name', { hasText: REPO_NAME });
  if (await existingTab.count()) {
    await existingTab.first().click();
  } else {
    await page.locator('.git-tab-new').click();
    const menu = page.locator('.ctx-menu');
    await menu.locator('.ctx-search-input').fill(REPO_NAME);
    await menu.getByRole('menuitem', { name: REPO_NAME }).first().click();
  }
  await expect(page.locator('.refs-panel')).toBeVisible({ timeout: 30_000 });
  await expect(page.locator('.graph-row[data-sha]').first()).toBeVisible({ timeout: 30_000 });
  // Let the first history page's layout settle before measuring anything.
  await page.waitForLoadState('networkidle').catch(() => {});
}

function branchRow(page: Page, name: string) {
  return page
    .locator('.refs-panel .ref-row:not(.remote):not(.tag):not(.stash-row)')
    .filter({ has: page.locator('.ref-name', { hasText: new RegExp(`^\\s*${name}\\s*$`) }) })
    .first();
}

/** Start collecting long tasks (Chromium `longtask` entries) from now on. */
async function watchLongTasks(page: Page): Promise<void> {
  // Without `longtask` support the max-long-task assertion would pass
  // vacuously (0 entries) — fail loudly instead.
  const supported = await page.evaluate(() => PerformanceObserver.supportedEntryTypes.includes('longtask'));
  expect(supported, 'this browser reports longtask entries').toBe(true);
  await page.evaluate(() => {
    const w = window as unknown as { __lt: number[]; __ltObs?: PerformanceObserver };
    w.__lt = [];
    w.__ltObs?.disconnect();
    w.__ltObs = new PerformanceObserver((list) => {
      for (const e of list.getEntries()) w.__lt.push(e.duration);
    });
    w.__ltObs.observe({ type: 'longtask', buffered: false });
  });
}

async function longTasks(page: Page): Promise<number[]> {
  return page.evaluate(() => (window as unknown as { __lt: number[] }).__lt ?? []);
}

/** Count "ResizeObserver loop …" reports from now on (console + page
 *  errors): a measurer that re-lays out inside its own RO callback spams it
 *  and pays an extra layout pass per frame. */
function watchResizeLoops(page: Page): () => number {
  let n = 0;
  const hit = (t: string): void => {
    if (/ResizeObserver loop/i.test(t)) n++;
  };
  page.on('console', (m) => hit(m.text()));
  page.on('pageerror', (e) => hit(e.message));
  return () => n;
}

function apiPath(r: Request): string {
  const u = new URL(r.url());
  return u.pathname + u.search;
}

test('single click on a 100k-line branch paints the file list fast and mounts no diff rows', async ({ page }) => {
  await openRepo(page);
  const diffReqs: string[] = [];
  page.on('request', (r) => {
    if (r.method() === 'GET' && /\/repos\/[^/]+\/diff\?/.test(r.url())) diffReqs.push(apiPath(r));
  });
  await watchLongTasks(page);
  const resizeLoops = watchResizeLoops(page);

  const t0 = Date.now();
  await branchRow(page, 'huge').click();
  await expect(page.locator('.detail-diff .df-block').first()).toBeVisible({ timeout: 10_000 });
  const fileListMs = Date.now() - t0;
  // Settle: anything the click kicked off (lazy fetches, layout) has landed.
  await page.waitForTimeout(1500);

  const rows = await page.locator('.detail-diff tr').count();
  const blocks = await page.locator('.detail-diff .df-block').count();
  const lts = await longTasks(page);
  const maxLt = lts.length ? Math.max(...lts) : 0;
  console.log(
    `[perf] huge click: file list ${fileListMs} ms · diff rows ${rows} · file blocks ${blocks} · ` +
      `long tasks ${lts.length} (max ${Math.round(maxLt)} ms) · diff requests ${JSON.stringify(diffReqs)}`,
  );

  // GIT-5 target: the list paints ≤ 300 ms after the click even though the
  // pane itself waits out the 200 ms double-click window — its summary is
  // prefetched at the click (`prefetchCommitSummary`), so the list renders
  // from cache when the window closes (was 380–390 ms: window + round-trip).
  expect(fileListMs, 'file list painted within 300 ms of the click').toBeLessThanOrEqual(300);
  expect(rows, 'diff rows in the DOM after the click').toBeLessThan(2000);
  expect(blocks, 'file headers are paged, not all 400 mounted').toBeLessThanOrEqual(200);
  expect(maxLt, 'no main-thread task over 200 ms').toBeLessThan(200);
  // The click asked for the summary only — never the whole patch.
  expect(diffReqs.length).toBeGreaterThan(0);
  expect(diffReqs.every((u) => /summary=true|[?&]path=/.test(u)), diffReqs.join('\n')).toBe(true);
  await expect(page.locator('.diff-summary-bar')).toContainText('400 files');

  // Opening one file fetches just that file and renders its rows (capped).
  const before = diffReqs.length;
  await page.locator('.detail-diff .df-head').first().click();
  await expect(page.locator('.detail-diff tr').first()).toBeVisible({ timeout: 10_000 });
  expect(diffReqs.slice(before).some((u) => /[?&]path=/.test(u))).toBe(true);
  expect(await page.locator('.detail-diff tr').count()).toBeLessThanOrEqual(HUGE_LINES + 10);
  expect(resizeLoops(), 'no "ResizeObserver loop" errors').toBe(0);
});

test('double-click (checkout) never requests the diff and does not re-read history', async ({ page }) => {
  await openRepo(page);
  const seen: string[] = [];
  page.on('request', (r) => {
    if (/\/api\/v1\/repos\//.test(r.url()) && r.method() !== 'OPTIONS') seen.push(`${r.method()} ${apiPath(r)}`);
  });

  await branchRow(page, 'huge').dblclick();
  await expect(branchRow(page, 'huge')).toHaveClass(/current/, { timeout: 20_000 });
  await page.waitForTimeout(1000); // past the deferred-diff window

  console.log(`[perf] huge dblclick requests: ${JSON.stringify(seen)}`);
  expect(seen.some((s) => s.startsWith('POST') && s.includes('/checkout'))).toBe(true);
  // The lead click may warm the file-list SUMMARY (one small numstat read,
  // aborted by the double-click) — but a checkout gesture never asks for a
  // patch: no full diff, no per-file diff.
  const diffs = seen.filter((s) => s.includes('/diff?'));
  expect(diffs.filter((s) => !/summary=true/.test(s)), 'no patch request for a checkout gesture').toEqual([]);
  expect(diffs.length, 'at most the one prefetched summary').toBeLessThanOrEqual(1);
  expect(seen.filter((s) => s.includes('/log?')), 'a checkout does not re-read the log').toEqual([]);
  // The pane offers the diff on demand instead.
  await expect(page.getByRole('button', { name: 'Show this commit’s changes' })).toBeVisible();
});

test('a Fetch that moves nothing does not re-read history (old branch tip beyond the first page)', async ({ page }) => {
  await openRepo(page);
  // The stale branch is in the sidebar but its tip is NOT in the loaded page.
  await expect(branchRow(page, 'stale')).toBeVisible();
  const seen: string[] = [];
  page.on('request', (r) => {
    if (/\/api\/v1\/repos\//.test(r.url()) && r.method() === 'GET') seen.push(apiPath(r));
  });
  const refsReq = page.waitForRequest((r) => /\/repos\/[^/]+\/refs(\?|$)/.test(r.url()) && r.method() === 'GET', {
    timeout: 20_000,
  });
  await page.getByRole('button', { name: 'Fetch and pull options', exact: true }).click(); await page.getByRole('menuitem', { name: 'Fetch', exact: true }).click();
  await refsReq; // the post-fetch re-sync ran its cheap refs check…
  await page.waitForLoadState('networkidle').catch(() => {});
  await page.waitForTimeout(500);
  console.log(`[perf] no-op fetch requests: ${JSON.stringify(seen)}`);
  // …and decided nothing moved.
  expect(seen.filter((s) => s.includes('/log?')), 'a no-op Fetch does not re-read the log').toEqual([]);
});

test('repeated refreshes do not grow the history request', async ({ page }) => {
  await openRepo(page);
  const limits: number[] = [];
  page.on('request', (r) => {
    const m = /\/log\?[^#]*\blimit=(\d+)/.exec(r.url());
    if (m && r.method() === 'GET') limits.push(Number(m[1]));
  });

  for (let i = 0; i < 3; i++) {
    // Move a branch outside the app, then Fetch: the re-sync sees history
    // moved and re-reads the log.
    git(repoDir, 'commit', '-q', '--allow-empty', '-m', `external ${i}`);
    const logReq = page.waitForRequest((r) => /\/log\?/.test(r.url()) && r.method() === 'GET', { timeout: 20_000 });
    await page.getByRole('button', { name: 'Fetch and pull options', exact: true }).click(); await page.getByRole('menuitem', { name: 'Fetch', exact: true }).click();
    await logReq;
    await page.waitForLoadState('networkidle').catch(() => {});
  }

  console.log(`[perf] log limits across refreshes: ${JSON.stringify(limits)}`);
  expect(limits.length).toBeGreaterThanOrEqual(3);
  // One page of history was loaded; every refresh re-reads exactly that much.
  expect(Math.max(...limits)).toBeLessThanOrEqual(10_000);
});

test('context menus opened on the Git page do not pin it after leaving (heap probe)', async ({ page }) => {
  test.setTimeout(120_000);
  // Baseline: Home, after the Git page has been visited once (its module
  // caches — diff summaries, graph snapshots — are legitimately retained).
  await openRepo(page);
  await openPage(page, 'home');
  const baseline = await heapAfterGC(page);

  await openRepo(page);
  const row = page.locator('.graph-row[data-sha] .graph-select').first();
  const menu = page.locator('.ctx-menu');
  for (let i = 0; i < 50; i++) {
    await row.click({ button: 'right' });
    await expect(menu).toBeVisible();
    await page.keyboard.press('Escape');
    await expect(menu).toBeHidden();
  }
  await openPage(page, 'home');
  const after = await heapAfterGC(page);
  const mb = (b: number) => Math.round(b / (1 << 20));
  console.log(`[perf] ctxMenu heap probe: baseline ${mb(baseline)} MB → after 50 menus + leave ${mb(after)} MB`);
  // R2-02: the menu store used to keep the last trigger element and every
  // item closure alive, which pinned the whole Git page (≈159 MB).
  expect(after, 'heap after leaving Git ≤ baseline + 20 MB').toBeLessThanOrEqual(baseline + 20 * (1 << 20));
});
