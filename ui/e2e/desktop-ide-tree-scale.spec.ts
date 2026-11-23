import { test, expect, type Page } from '@playwright/test';
import { mkdirSync, mkdtempSync, realpathSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';

// A big project stays quick: ~5,000 folders and ~40,000 files, nested five deep, two folders
// holding 2,000 entries each. Only the folders the person has open are listed; folders that
// come back open after a reload are listed lazily, without blocking the first paint; coming
// back to the IDE or to the window lists nothing a watcher already covers; rows are
// virtualised. IDE_SCALE_OUT=<file> writes the measured numbers as JSON.
// Needs a daemon built from this tree (OTTO_E2E_BIN=<repo>/target/debug/ottod).

test.use({ serviceWorkers: 'block', viewport: { width: 1280, height: 800 } });

let root = '';
const OPEN: string[] = [];
const out: Record<string, unknown> = {};

function buildFixture(): void {
  root = realpathSync(mkdtempSync(join(tmpdir(), 'otto-e2e-ide-scale-')));
  let dirs = 0;
  let files = 0;
  const dir = (p: string): void => { mkdirSync(p, { recursive: true }); dirs++; };
  const put = (p: string): void => { writeFileSync(p, 'x\n'); files++; };
  put(join(root, 'README.md'));
  for (let a = 0; a < 10; a++) {
    for (let b = 0; b < 20; b++) {
      for (let c = 0; c < 24; c++) {
        const leaf = join(root, `pkg${a}`, `sub${b}`, `leaf${c}`);
        dir(leaf);
        for (let f = 0; f < 8; f++) put(join(leaf, `f${f}.ts`));
        if (c === 0) { dir(join(leaf, 'deep', 'deeper')); put(join(leaf, 'deep', 'deeper', 'd.ts')); }
      }
    }
  }
  for (const big of ['big-a', 'big-b']) {
    dir(join(root, big));
    for (let i = 0; i < 2000; i++) put(join(root, big, `entry${String(i).padStart(4, '0')}.ts`));
  }
  out.fixture = { dirs, files };
  // What the person had open: two levels in three packages, a deep chain, both big folders.
  for (let a = 0; a < 3; a++) {
    OPEN.push(`pkg${a}`);
    for (let b = 0; b < 6; b++) OPEN.push(`pkg${a}/sub${b}`);
  }
  OPEN.push('pkg0/sub0/leaf0', 'pkg0/sub0/leaf0/deep', 'pkg0/sub0/leaf0/deep/deeper', 'big-a', 'big-b');
}

interface Probe { calls: { url: string; bytes: number; ms: number }[]; }

async function instrument(page: Page): Promise<Probe> {
  const probe: Probe = { calls: [] };
  const started = new Map<string, number>();
  page.on('request', (r) => { if (r.url().includes('/ide/fs/tree')) started.set(r.url() + r.timing().startTime, Date.now()); });
  page.on('requestfinished', async (r) => {
    if (!r.url().includes('/ide/fs/tree')) return;
    const sizes = await r.sizes().catch(() => ({ responseBodySize: 0 }));
    const t = r.timing();
    probe.calls.push({ url: decodeURIComponent(r.url().split('?')[1] ?? ''), bytes: sizes.responseBodySize, ms: Math.round(t.responseEnd) });
  });
  await page.addInitScript(() => {
    localStorage.setItem('otto_rail_expanded', '0');
    localStorage.setItem('otto.ide.agentOpen', '0');
    const w = window as unknown as { __longTasks: number[] };
    w.__longTasks = [];
    try {
      new PerformanceObserver((l) => { for (const e of l.getEntries()) w.__longTasks.push(Math.round(e.duration)); }).observe({ type: 'longtask', buffered: true });
    } catch { /* not supported */ }
  });
  return probe;
}

// The first row under the root (folders sort first): the tree is usable once it shows.
const rootRow = (page: Page) => page.locator(`[data-tree-key="${join(root, 'big-a')}"]`);
const longTasks = (page: Page) => page.evaluate(() => (window as unknown as { __longTasks: number[] }).__longTasks.splice(0));
const domRows = (page: Page) => page.getByTestId('ide-project-tree').locator('.tree-row').count();

async function seedOpenFolders(page: Page): Promise<void> {
  await page.evaluate(({ key, expanded }) => {
    const map = JSON.parse(localStorage.getItem('otto.ide.projectMemory') ?? '{}');
    map[key] = { expanded, at: Date.now() };
    localStorage.setItem('otto.ide.projectMemory', JSON.stringify(map));
  }, { key: `local::${root}`, expanded: OPEN.map((p) => join(root, p)) });
}

test.describe('Desktop IDE tree: a big project', () => {
  test.describe.configure({ mode: 'serial' });
  test.beforeAll(() => buildFixture());
  test.afterAll(() => {
    const file = process.env.IDE_SCALE_OUT;
    if (file) writeFileSync(file, JSON.stringify(out, null, 2));
    if (root) rmSync(root, { recursive: true, force: true });
  });
  test.beforeEach(({}, info) => {
    test.skip(info.project.name !== 'desktop-browser', 'desktop-browser only');
  });

  test('opens, reloads with folders open, comes back and refocuses without listing the world', async ({ page }) => {
    test.setTimeout(120_000);
    const probe = await instrument(page);

    // 1. Cold open, nothing remembered.
    let t0 = Date.now();
    await page.goto(`/#/ide?root=${encodeURIComponent(root)}`);
    await expect(rootRow(page)).toBeVisible({ timeout: 30_000 });
    out.coldOpen = { firstTreeMs: Date.now() - t0, treeCalls: probe.calls.length, longTasks: await longTasks(page) };

    // 2. A reload of a project with 26 folders open (two of 2,000 entries).
    // (The reload lands on the default project; leaving this one saved its tree on pagehide.)
    await page.reload();
    await seedOpenFolders(page);
    probe.calls.length = 0;
    t0 = Date.now();
    await page.evaluate((p) => { location.hash = `#/ide?root=${encodeURIComponent(p)}`; }, root);
    await expect(rootRow(page)).toBeVisible({ timeout: 30_000 });
    const firstTreeMs = Date.now() - t0;
    await expect(page.locator(`[data-tree-key="${join(root, 'big-a/entry0000.ts')}"]`)).toBeVisible({ timeout: 30_000 });
    const firstOpenFolderMs = Date.now() - t0;
    await page.waitForTimeout(2_500); // ui-guards: allow — absence: no more listings arrive
    const calls = probe.calls.splice(0);
    out.reloadWithOpenFolders = {
      firstTreeMs, firstOpenFolderMs,
      treeCalls: calls.length,
      treeBytes: calls.reduce((s, c) => s + c.bytes, 0),
      slowestCallMs: Math.max(0, ...calls.map((c) => c.ms)),
      duplicateListings: calls.length - new Set(calls.map((c) => c.url)).size,
      domRows: await domRows(page),
      longTasks: await longTasks(page),
    };

    // Only what is on screen is listed (both big folders fill the view); a folder is read again
    // at most once, when the watcher's lease lands (the gap between its listing and the lease).
    expect((out.reloadWithOpenFolders as { duplicateListings: number }).duplicateListings).toBeLessThanOrEqual(3);
    expect(calls.length).toBeLessThanOrEqual(8);

    // 3. Leave the IDE and come back.
    await page.evaluate(() => { location.hash = '#/home'; });
    await expect(page.getByTestId('ide-project-tree')).toHaveCount(0, { timeout: 10_000 });
    t0 = Date.now();
    await page.evaluate(() => { location.hash = '#/ide'; });
    await expect(rootRow(page)).toBeVisible({ timeout: 30_000 });
    const backMs = Date.now() - t0;
    await page.waitForTimeout(2_500); // ui-guards: allow — absence: no more listings arrive
    const back = probe.calls.splice(0);
    out.returnToIde = { firstTreeMs: backMs, treeCalls: back.length, treeBytes: back.reduce((s, c) => s + c.bytes, 0), longTasks: await longTasks(page) };
    expect(back.length).toBeLessThanOrEqual(4);

    // 4. Back to the window (the watcher covers the root).
    await page.evaluate(() => window.dispatchEvent(new Event('focus')));
    await page.waitForTimeout(2_500); // ui-guards: allow — absence: no more listings arrive
    out.windowFocus = { treeCalls: probe.calls.splice(0).length };
    expect(out.windowFocus).toEqual({ treeCalls: 0 });

    // 5. A 2,000-entry folder scrolls; rows stay virtualised.
    const big = page.locator(`[data-tree-key="${join(root, 'big-a')}"]`);
    await big.scrollIntoViewIfNeeded();
    const scroller = page.getByTestId('ide-project-tree').locator('.tree-scroll');
    await scroller.evaluate((el) => { el.scrollTop = el.scrollHeight / 2; });
    await expect.poll(() => domRows(page)).toBeGreaterThan(0);
    out.scrolled = { domRows: await domRows(page), scrollHeight: await scroller.evaluate((el) => el.scrollHeight) };
    expect(out.scrolled).toMatchObject({ domRows: expect.any(Number) });
    expect(await domRows(page)).toBeLessThan(150);

    // 6. Past the big folders, the folders that came back open load as they come into view.
    // (root + big-a + 2,000 + big-b + 2,000 rows of 26 px above pkg0)
    await scroller.evaluate((el) => { el.scrollTop = 4_003 * 26; });
    await expect(page.locator(`[data-tree-key="${join(root, 'pkg0/sub0/leaf0')}"]`)).toBeVisible({ timeout: 10_000 });
    await expect(page.locator(`[data-tree-key="${join(root, 'pkg0/sub0/leaf0/f0.ts')}"]`)).toBeVisible({ timeout: 10_000 });
    out.lazy = { treeCalls: probe.calls.splice(0).length };
    // eslint-disable-next-line no-console
    console.log('IDE_SCALE', JSON.stringify(out));
  });

  test('this repository opens and its folders expand quickly', async ({ page }) => {
    const repo = realpathSync(join(process.cwd(), '..'));
    const probe = await instrument(page);
    const t0 = Date.now();
    await page.goto(`/#/ide?root=${encodeURIComponent(repo)}`);
    const at = (p: string) => page.locator(`[data-tree-key="${join(repo, p)}"]`);
    await expect(at('crates')).toBeVisible({ timeout: 30_000 });
    const firstTreeMs = Date.now() - t0;
    const expands: Record<string, number> = {};
    for (const [dir, child] of [['crates', 'crates/otto-server'], ['crates/otto-server', 'crates/otto-server/src'], ['crates/otto-server/src', 'crates/otto-server/src/routes']]) {
      const s = Date.now();
      await at(dir).click();
      await expect(at(child)).toBeVisible({ timeout: 10_000 });
      expands[dir] = Date.now() - s;
    }
    out.repo = { firstTreeMs, expandMs: expands, treeCalls: probe.calls.length, slowestCallMs: Math.max(0, ...probe.calls.map((c) => c.ms)), longTasks: await longTasks(page) };
    // eslint-disable-next-line no-console
    console.log('IDE_SCALE_REPO', JSON.stringify(out.repo));
  });
});
