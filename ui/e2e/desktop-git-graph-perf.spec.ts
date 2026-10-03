import { test, expect, type Page } from '@playwright/test';
import { execFileSync } from 'node:child_process';
import { mkdtempSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { apiCtx, seedWorkspace } from './seed';

// ─────────────────────────────────────────────────────────────────────────────
// PERF GATE — the commit graph on a 12k-commit, 20-lane history (desktop
// Chromium only). History is SYNTHESISED at the route layer (a real repo
// backs the page; `/log?all=true` and `/refs` are answered from a generated
// DAG) so the gate costs no fixture build. It locks down:
//   • first paint of the first 2k-commit page < 1 s after the page arrives,
//     and no main-thread task over 200 ms (first layout, prefetch, scroll,
//     load more, reload);
//   • the next 10k page is prefetched right after the first paint, and it
//     (like "load more") lays out ONLY the appended rows (resumable lane
//     layout, probed via window.__ottoGraphLayoutRows);
//   • a reload after a ref moved reads ONE first-size page and splices it
//     onto the held tail — never `max(PAGE, loaded)` (12k+) commits again —
//     and the layout converges with the previous pass a checkpoint below the
//     page instead of re-laying all 12k rows.
// ─────────────────────────────────────────────────────────────────────────────

test.use({ serviceWorkers: 'block' });
test.skip(({ isMobile, browserName }) => isMobile || browserName !== 'chromium', 'desktop Chromium perf gate');

const TOTAL = 12_000;
const LANES = 20;
const PAGE = 10_000;
const FIRST_PAGE = 2_000;

let repoId = '';
let workspaceId = '';

interface C { sha: string; short_sha: string; author: string; date: string; subject: string; parents: string[]; refs: string[] }

const sha = (n: number): string => n.toString(16).padStart(40, '0');

/** Newest-first DAG: commit i sits in lane i % LANES, its first parent is the
 *  next commit of its lane (i + LANES); every 97th commit also merges the
 *  neighbouring lane, so lanes converge/diverge like a busy repo. `extra`
 *  newer commits sit on lane 0 above everything (a "ref move"). */
function history(extra: number): C[] {
  const out: C[] = [];
  const t0 = Date.parse('2026-09-01T00:00:00Z');
  for (let e = extra; e >= 1; e--) {
    const s = `e${String(e).padStart(39, '0')}`;
    const parent = e > 1 ? `e${String(e - 1).padStart(39, '0')}` : sha(0);
    out.push({ sha: s, short_sha: s.slice(0, 7), author: 'perf', date: new Date(t0 + e * 1000).toISOString(), subject: `new ${e}`, parents: [parent], refs: [] });
  }
  for (let i = 0; i < TOTAL; i++) {
    const parents: string[] = [];
    if (i + LANES < TOTAL) parents.push(sha(i + LANES));
    if (i % 97 === 0 && i + 1 < TOTAL) parents.push(sha(i + 1));
    out.push({
      sha: sha(i),
      short_sha: sha(i).slice(-7),
      author: `dev${i % 7}`,
      date: new Date(t0 - i * 60_000).toISOString(),
      subject: `commit ${i} on lane ${i % LANES}`,
      parents,
      refs: i < LANES ? [`branch-${i}`] : [],
    });
  }
  return out;
}

test.beforeAll(async () => {
  const { ctx, base } = await apiCtx();
  workspaceId = await seedWorkspace(ctx, base);
  const dir = mkdtempSync(join(tmpdir(), 'otto-e2e-graph-perf-'));
  const git = (...a: string[]) => execFileSync('git', ['-C', dir, ...a], { stdio: 'ignore' });
  git('init', '-q', '-b', 'main');
  git('config', 'user.email', 'e2e@otto.local');
  git('config', 'user.name', 'E2E');
  git('config', 'commit.gpgsign', 'false');
  writeFileSync(join(dir, 'a.txt'), 'a\n');
  git('add', '.');
  git('commit', '-qm', 'init');
  const r = await ctx.post(`${base}/api/v1/workspaces/${workspaceId}/repos`, { data: { path: dir, name: 'graph-perf' } });
  expect(r.ok()).toBeTruthy();
  repoId = (await r.json()).id;
  await ctx.dispose();
});

test.beforeEach(async ({ page }) => {
  await page.addInitScript((id) => {
    localStorage.setItem('otto_workspace', id);
    localStorage.setItem('otto_rail_expanded', '0');
    localStorage.setItem('otto_git_auto_fetch', JSON.stringify({ enabled: false }));
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

const maxLongTask = (page: Page) => page.evaluate(() => Math.max(0, ...(window as unknown as { __lt: number[] }).__lt));
const resetLongTasks = (page: Page) => page.evaluate(() => ((window as unknown as { __lt: number[] }).__lt = []));
const layoutRows = (page: Page) => page.evaluate(() => (window as unknown as { __ottoGraphLayoutRows?: number }).__ottoGraphLayoutRows ?? -1);

test(`graph of ${TOTAL} commits × ${LANES} lanes: fast first paint, incremental load more, one-page splice on reload`, async ({ page }) => {
  test.setTimeout(120_000);
  let extra = 0;
  let firstPageAt = 0;
  const logLimits: { limit: number; skip: number }[] = [];
  await page.route('**/api/v1/repos/*/log?*', async (route) => {
    const u = new URL(route.request().url());
    if (u.searchParams.get('all') !== 'true') return route.fallback();
    const limit = Number(u.searchParams.get('limit') ?? PAGE);
    const skip = Number(u.searchParams.get('skip') ?? 0);
    logLimits.push({ limit, skip });
    const body = history(extra).slice(skip, skip + limit);
    if (!firstPageAt) {
      // Measure the graph, not app boot: drop long tasks logged so far.
      await resetLongTasks(page).catch(() => {});
      firstPageAt = Date.now();
    }
    await route.fulfill({ json: body });
  });
  await page.route('**/api/v1/repos/*/refs', async (route) => {
    const res = await route.fetch();
    const real = await res.json();
    const local = Array.from({ length: LANES }, (_, i) => ({
      name: `branch-${i}`,
      sha: i === 0 && extra > 0 ? `e${String(extra).padStart(39, '0')}` : sha(i),
      is_current: false,
      upstream: null,
      remote: false,
    }));
    await route.fulfill({ json: { ...real, local: [...real.local, ...local] } });
  });
  // The toolbar Fetch is the "a ref moved" trigger; the repo has no remote,
  // so answer it with the live status (a successful fetch bumps refsRev).
  await page.route('**/api/v1/repos/*/fetch', async (route) => {
    if (route.request().method() !== 'POST') return route.fallback();
    const res = await route.fetch({ method: 'GET', url: route.request().url().replace(/\/fetch$/, '/status') });
    await route.fulfill({ json: await res.json() });
  });

  await page.goto(`/#/git/${repoId}/graph`);
  await expect(page.locator('.graph-row[data-sha]').first()).toBeVisible({ timeout: 30_000 });
  const paintMs = Date.now() - firstPageAt;
  await page.waitForTimeout(300);
  console.log(`[perf] graph first paint ${paintMs} ms after the first page · max long task ${Math.round(await maxLongTask(page))} ms`);
  expect(paintMs, 'first page painted within 1 s of arriving').toBeLessThan(1000);
  expect(logLimits[0]).toEqual({ limit: FIRST_PAGE, skip: 0 });

  // The next page is prefetched without any scroll; its layout resumes
  // (10k appended rows laid out, not 12k).
  await expect.poll(() => logLimits.length, { timeout: 15_000 }).toBeGreaterThanOrEqual(2);
  expect(logLimits[1]).toEqual({ limit: PAGE, skip: FIRST_PAGE });
  await expect.poll(() => layoutRows(page), { timeout: 15_000 }).toBe(TOTAL - FIRST_PAGE);
  await page.waitForTimeout(300);
  expect(await maxLongTask(page), 'first layout + 10k prefetch').toBeLessThan(200);
  // Windowed: 12k commits never mount 12k rows.
  expect(await page.locator('.graph-row[data-sha]').count()).toBeLessThan(200);

  // Load more: scroll to the bottom → one more request, which finds the root.
  await resetLongTasks(page);
  await page.locator('.graph-panel').evaluate((el) => (el.scrollTop = el.scrollHeight));
  await expect.poll(() => logLimits.length, { timeout: 15_000 }).toBeGreaterThanOrEqual(3);
  expect(logLimits[2]).toEqual({ limit: PAGE, skip: TOTAL });
  expect(await maxLongTask(page), 'load more + scroll').toBeLessThan(200);

  // A ref moves (one new commit on branch-0) → reload reads ONE page and
  // splices it onto the held tail.
  extra = 1;
  const before = logLimits.length;
  await resetLongTasks(page);
  await page.getByTitle('Fetch from remote').click();
  await expect.poll(() => logLimits.length, { timeout: 15_000 }).toBeGreaterThan(before);
  await page.waitForTimeout(500);
  const reloads = logLimits.slice(before);
  console.log(`[perf] reload after ref move: ${JSON.stringify(reloads)}`);
  expect(reloads.length, 'one history request per reload').toBe(1);
  expect(reloads[0].limit, 'reload reads one first-size page').toBe(FIRST_PAGE);
  // The splice converges with the previous layout a checkpoint (≤ 1k rows)
  // below the page — not a relayout of all 12k rows.
  await expect.poll(() => layoutRows(page), { timeout: 10_000 }).toBeLessThanOrEqual(FIRST_PAGE + 1_001);
  await expect(page.locator('.mob-sec-count, .graph-row[data-sha]').first()).toBeVisible();
  expect(await maxLongTask(page), 'reload + relayout').toBeLessThan(200);
  // The new commit is on top, and the paged-in tail survived the splice.
  await page.locator('.graph-panel').evaluate((el) => (el.scrollTop = 0));
  await expect(page.locator(`.graph-row[data-sha="e${'1'.padStart(39, '0')}"]`)).toBeVisible({ timeout: 10_000 });
  const total = await page.locator('.graph-panel').evaluate((el) => Math.round(el.scrollHeight));
  expect(total, 'scroll height still covers the whole spliced history').toBeGreaterThan(TOTAL * 20);
});
