import { test, expect, type Page } from '@playwright/test';
import { apiCtx, seedWorkspace } from './seed';
import { budgetMs, isDesktopProject } from './perf';

// ─────────────────────────────────────────────────────────────────────────────
// Boot / time-to-first-page budget (perf F1/F3). The shell marks
// `otto:shell-mounted` (its first effect) and `otto:page-painted` (after the
// frame that first paints a page) — shell/App.svelte. For a cold load of
// `#/agents` and `#/home` this spec budgets:
//  - time to shell-mounted and to page-painted (scaled with budgetMs);
//  - the NUMBER of daemon API requests started before the first page paint
//    (never scaled — a count, not a time);
//  - the boot overlap that removed the serial waterfall: `/auth/me` and
//    `/auth/capabilities` start before `/meta` has answered, and the scratch
//    workspace before `/workspaces` has.
// Everything is read from the page's own Resource Timing entries, so the
// numbers don't depend on Playwright's clock. The [boot-perf] log line carries
// the measured values for re-calibrating the ceilings below.
// ─────────────────────────────────────────────────────────────────────────────

/** Shell mounted (first effect), from navigation start. */
const SHELL_MOUNT_BUDGET_MS = 2_500;
/** First page painted, from navigation start. */
const PAGE_PAINT_BUDGET_MS = 3_500;
/** Daemon API requests started before the first page paint (a ceiling). */
const MAX_API_REQUESTS_BEFORE_PAINT = 24;

let wsId = '';

test.beforeAll(async () => {
  const { ctx, base } = await apiCtx();
  wsId = await seedWorkspace(ctx, base);
  await ctx.dispose();
});

test.beforeEach(async ({ page }, info) => {
  test.skip(!isDesktopProject(info.project.name), 'desktop projects only');
  await page.addInitScript((id) => {
    localStorage.setItem('otto_workspace', id as string);
    localStorage.setItem('otto_firstrun_dismissed', '1');
  }, wsId);
});

interface BootSample {
  shellMounted: number;
  pagePainted: number;
  apiBeforePaint: number;
  apiPaths: string[];
  overlap: { meStartedBeforeMetaEnd: boolean | null; capsStartedBeforeMetaEnd: boolean | null; scratchStartedBeforeListEnd: boolean | null };
}

async function bootSample(page: Page, hash: string): Promise<BootSample> {
  await page.goto(`/${hash}`);
  await page.waitForFunction(() => performance.getEntriesByName('otto:page-painted').length > 0, null, { timeout: 30_000 });
  return page.evaluate(() => {
    const mark = (n: string): number => performance.getEntriesByName(n)[0]?.startTime ?? -1;
    const painted = mark('otto:page-painted');
    // Daemon API calls: fetch/XHR to anything that isn't a source module or
    // an asset (the Vite dev server serves those; the daemon serves the rest).
    const api = (performance.getEntriesByType('resource') as PerformanceResourceTiming[]).filter((e) => {
      if (e.initiatorType !== 'fetch' && e.initiatorType !== 'xmlhttprequest') return false;
      const p = new URL(e.name).pathname;
      return !/^\/(src|assets|node_modules|@vite|@fs|@id)\//.test(p) && !/\.(js|ts|svelte|css|json|wasm|svg|png|woff2?)$/.test(p);
    });
    const byPath = (re: RegExp) => api.find((e) => re.test(new URL(e.name).pathname));
    const meta = byPath(/\/meta$/);
    const me = byPath(/\/auth\/me$/);
    const caps = byPath(/\/auth\/capabilities$/);
    const list = byPath(/\/workspaces$/);
    const scratch = byPath(/\/workspaces\/[^/]+$/);
    const before = (a?: PerformanceResourceTiming, b?: PerformanceResourceTiming) =>
      a && b ? a.startTime <= b.responseEnd : null;
    const early = api.filter((e) => e.startTime <= painted);
    return {
      shellMounted: mark('otto:shell-mounted'),
      pagePainted: painted,
      apiBeforePaint: early.length,
      apiPaths: early.map((e) => new URL(e.name).pathname),
      overlap: {
        meStartedBeforeMetaEnd: before(me, meta),
        capsStartedBeforeMetaEnd: before(caps, meta),
        scratchStartedBeforeListEnd: before(scratch, list),
      },
    };
  });
}

for (const hash of ['#/agents', '#/home']) {
  test(`cold boot of ${hash}: shell + first page within budget, bounded request count, no serial auth waterfall`, async ({ page }) => {
    test.setTimeout(90_000);
    // One warm-up load so the dev server has transformed every module; the
    // measured load is the second (what a person sees on every app start).
    await bootSample(page, hash);
    const s = await bootSample(page, hash);
    // eslint-disable-next-line no-console
    console.log(`[boot-perf] ${hash} ${JSON.stringify(s)}`);

    expect(s.shellMounted, 'shell never marked mounted').toBeGreaterThan(0);
    expect(s.shellMounted, `shell mounted ${s.shellMounted.toFixed(0)} ms after navigation start`).toBeLessThanOrEqual(budgetMs(SHELL_MOUNT_BUDGET_MS));
    expect(s.pagePainted, `first page painted ${s.pagePainted.toFixed(0)} ms after navigation start`).toBeLessThanOrEqual(budgetMs(PAGE_PAINT_BUDGET_MS));
    expect(s.apiBeforePaint, `API requests before first paint: ${s.apiPaths.join(', ')}`).toBeLessThanOrEqual(MAX_API_REQUESTS_BEFORE_PAINT);
    // F3: identity + grants go out with /meta, the scratch row with the list.
    expect(s.overlap.meStartedBeforeMetaEnd, '/auth/me waited for /meta').toBe(true);
    expect(s.overlap.capsStartedBeforeMetaEnd, '/auth/capabilities waited for /meta').toBe(true);
    expect(s.overlap.scratchStartedBeforeListEnd, 'the scratch workspace waited for /workspaces').toBe(true);
  });
}
