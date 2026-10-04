import { test, expect, type Page } from '@playwright/test';
import { apiCtx, seedWorkspace } from './seed';
import { budgetMs, isDesktopProject } from './perf';

// ─────────────────────────────────────────────────────────────────────────────
// Boot / time-to-first-page budget (perf F1/F3). The shell marks
// `otto:shell-mounted` (its first effect) and `otto:page-painted` (after the
// frame that first paints a page) — shell/App.svelte. For a cold load of
// `#/agents` and `#/home` this spec budgets:
//  - time to shell-mounted and to page-painted (scaled with budgetMs);
//  - NO DUPLICATE daemon API request (same method + path + query) during the
//    boot window — navigation until the network has been quiet for
//    BOOT_QUIET_MS after the first paint (capped at BOOT_WINDOW_MAX_MS);
//  - the TOTAL number of distinct daemon API requests in that window (a count,
//    never scaled). Both read a fetch wrapper in the page, so they don't depend
//    on when the page happens to paint: the old "requests started before first
//    paint" count grew on a slower machine (12 locally, 18 on CI's WebKit for
//    the same code) and is now logged for information only;
//  - the boot overlap that removed the serial waterfall: `/auth/me` and
//    `/auth/capabilities` start before `/meta` has answered, and the scratch
//    workspace and the saved workspace's session list (perf G3) before
//    `/workspaces` has.
// Everything is read from the page's own Resource Timing entries, so the
// numbers don't depend on Playwright's clock. The [boot-perf] log line carries
// the measured values for re-calibrating the ceilings below.
// ─────────────────────────────────────────────────────────────────────────────

// Ceilings = the worst measured value ×1.5 (2026-10-03, merged perf wave, Vite
// dev server, desktop-browser, 2 workers on a loaded machine): #/agents shell
// 2636 ms / paint 2715 ms (1177 / 1204 with one worker), #/home 931 / 1008 ms;
// The distinct-request ceilings are the measured window total + 3 (2026-10-04,
// after the boot de-dupe): #/agents 20 and #/home 33 on BOTH desktop-webkit
// and desktop-browser, 0 duplicates — a set of distinct requests doesn't drift
// with load, only with code. (The same build counted 12 vs 18 "before paint".)
/** Shell mounted (first effect), from navigation start. */
const SHELL_MOUNT_BUDGET_MS = 4_000;
/** First page painted, from navigation start. */
const PAGE_PAINT_BUDGET_MS = 4_100;
/** Distinct daemon API requests in the boot window, per page (a ceiling). */
const MAX_DISTINCT_BOOT_REQUESTS: Record<string, number> = { '#/agents': 23, '#/home': 36 };
/** The boot window ends once no new API request has started for this long after paint. */
const BOOT_QUIET_MS = 1_500;
/** …or at this point after navigation, whichever comes first. */
const BOOT_WINDOW_MAX_MS = 15_000;

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
  /** Every daemon API request in the boot window, as `METHOD path?query`. */
  bootRequests: string[];
  overlap: {
    meStartedBeforeMetaEnd: boolean | null;
    capsStartedBeforeMetaEnd: boolean | null;
    scratchStartedBeforeListEnd: boolean | null;
    sessionsStartedBeforeListEnd: boolean | null;
  };
}

async function bootSample(page: Page, hash: string): Promise<BootSample> {
  // The dev server serves every module as its own resource: #/agents loads
  // more than the default 250-entry Resource Timing buffer holds, which
  // silently dropped every API entry after the first few. Make room first.
  await page.addInitScript(() => performance.setResourceTimingBufferSize(20_000));
  // Recorded in the page (a fetch wrapper), not from Playwright's request
  // events: Chromium routes the app's fetches through its service worker,
  // and those never surface as page requests. The app's own calls are fetch.
  await page.addInitScript(() => {
    const w = window as unknown as { __bootReqs?: { key: string; at: number }[] };
    // Each bootSample() adds this script again; wrap fetch once per document.
    if (w.__bootReqs) return;
    const reqs: { key: string; at: number }[] = [];
    w.__bootReqs = reqs;
    const orig = window.fetch.bind(window);
    window.fetch = (input: RequestInfo | URL, init?: RequestInit) => {
      const req = input instanceof Request ? input : null;
      const u = new URL(req ? req.url : String(input), location.href);
      const method = (init?.method ?? req?.method ?? 'GET').toUpperCase();
      reqs.push({ key: `${method} ${u.pathname}${u.search}`, at: performance.now() });
      return orig(input, init);
    };
  });
  await page.goto(`/${hash}`);
  await page.waitForFunction(() => performance.getEntriesByName('otto:page-painted').length > 0, null, { timeout: 30_000 });
  // A page can paint before the workspace list goes out (#/agents paints
  // after 3 requests): wait for the list + scratch row to finish so their
  // overlap is measured, not reported as "unknown". Counts stay paint-relative.
  await page.waitForFunction(
    (id) => {
      const paths = (performance.getEntriesByType('resource') as PerformanceResourceTiming[]).map((e) => new URL(e.name).pathname);
      return (
        paths.some((p) => /\/api\/v1\/workspaces$/.test(p)) &&
        paths.some((p) => /\/api\/v1\/workspaces\/[^/]+$/.test(p)) &&
        paths.some((p) => p.endsWith(`/api/v1/workspaces/${id}/sessions`))
      );
    },
    wsId,
    { timeout: 15_000 },
  );
  // The boot window: wait until no new API request has started for
  // BOOT_QUIET_MS (pollers on 15 s+ cadences never fire inside it).
  const bootRequests = await page.evaluate(
    async ({ quiet, max }) => {
      const w = window as unknown as { __bootReqs?: { key: string; at: number }[] };
      const api = () => (w.__bootReqs ?? []).filter((r) => r.key.split(' ')[1].startsWith('/api/'));
      for (;;) {
        const reqs = api();
        const last = reqs.length ? reqs[reqs.length - 1].at : 0;
        const now = performance.now();
        if (now - last >= quiet || now >= max) return api().map((r) => r.key);
        await new Promise((r) => setTimeout(r, 100));
      }
    },
    { quiet: BOOT_QUIET_MS, max: BOOT_WINDOW_MAX_MS },
  );
  const sample = await page.evaluate((id) => {
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
    // The first request for the saved workspace's sessions (the speculative
    // shown-list one; the archived probe shares the path but starts later).
    const sessions = api.find((e) => new URL(e.name).pathname.endsWith(`/workspaces/${id}/sessions`));
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
        sessionsStartedBeforeListEnd: before(sessions, list),
      },
    };
  }, wsId);
  return { ...sample, bootRequests };
}

/** Request keys issued more than once, with their counts. */
function duplicates(keys: string[]): string[] {
  const n = new Map<string, number>();
  for (const k of keys) n.set(k, (n.get(k) ?? 0) + 1);
  return [...n].filter(([, c]) => c > 1).map(([k, c]) => `${k} ×${c}`);
}

for (const hash of ['#/agents', '#/home']) {
  test(`cold boot of ${hash}: shell + first page within budget, no duplicate requests, bounded request count, no serial auth waterfall`, async ({ page }) => {
    test.setTimeout(90_000);
    // One warm-up load so the dev server has transformed every module; the
    // measured load is the second (what a person sees on every app start).
    await bootSample(page, hash);
    const s = await bootSample(page, hash);
    // eslint-disable-next-line no-console
    console.log(`[boot-perf] ${hash} ${JSON.stringify({ ...s, distinct: new Set(s.bootRequests).size })}`);

    expect(s.shellMounted, 'shell never marked mounted').toBeGreaterThan(0);
    expect(s.shellMounted, `shell mounted ${s.shellMounted.toFixed(0)} ms after navigation start`).toBeLessThanOrEqual(budgetMs(SHELL_MOUNT_BUDGET_MS));
    expect(s.pagePainted, `first page painted ${s.pagePainted.toFixed(0)} ms after navigation start`).toBeLessThanOrEqual(budgetMs(PAGE_PAINT_BUDGET_MS));
    // A count read off the request events, not the paint mark: deterministic
    // for a given build, so a regression is a duplicate load or a new request.
    expect(duplicates(s.bootRequests), 'duplicate API requests during boot').toEqual([]);
    const distinct = new Set(s.bootRequests).size;
    expect(distinct, `distinct API requests during boot: ${[...new Set(s.bootRequests)].join(', ')}`).toBeLessThanOrEqual(MAX_DISTINCT_BOOT_REQUESTS[hash]);
    // F3: identity + grants go out with /meta, the scratch row with the list.
    expect(s.overlap.meStartedBeforeMetaEnd, '/auth/me waited for /meta').toBe(true);
    expect(s.overlap.capsStartedBeforeMetaEnd, '/auth/capabilities waited for /meta').toBe(true);
    expect(s.overlap.scratchStartedBeforeListEnd, 'the scratch workspace waited for /workspaces').toBe(true);
    // G3: the saved workspace's session list goes out with /workspaces too.
    expect(s.overlap.sessionsStartedBeforeListEnd, "the saved workspace's sessions waited for /workspaces").toBe(true);
  });
}
