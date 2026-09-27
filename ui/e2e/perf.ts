// Shared e2e perf probes (GAPS_TO_9_5 §0 "E"). One copy of the measuring code
// every perf spec needs, so budgets read the same way everywhere.
//
// Rules for a perf spec (see desktop-db-editor-perf.spec.ts):
// - `test.skip(isMobile || project !== 'desktop-browser')`.
// - WebKit for keystroke/frame budgets (`frameDeltas`); Chromium for long-task
//   and heap budgets (`watchLongTasks`, `heapAfterGC` — both assert support,
//   so a check is never vacuous on the wrong engine).
// - Budget DOM/request COUNTS first, timings second: counts don't flake.

import { expect, type Page, type Request } from '@playwright/test';

/** p-th percentile (nearest-rank) of `xs`; 0 for an empty list. */
export function percentile(xs: readonly number[], p: number): number {
  if (xs.length === 0) return 0;
  const s = [...xs].sort((a, b) => a - b);
  const i = Math.min(s.length - 1, Math.max(0, Math.ceil((p / 100) * s.length) - 1));
  return s[i];
}

export interface Dist {
  n: number;
  p50: number;
  p95: number;
  max: number;
}

export function dist(xs: readonly number[]): Dist {
  return { n: xs.length, p50: percentile(xs, 50), p95: percentile(xs, 95), max: xs.length ? Math.max(...xs) : 0 };
}

// ── long tasks (Chromium) ─────────────────────────────────────────────────────

/** Start collecting `longtask` entries from now on (replaces a previous watch).
 *  Fails when the engine has no `longtask` support — a 0-entry pass there
 *  would be vacuous. */
export async function watchLongTasks(page: Page): Promise<void> {
  const supported = await page.evaluate(() => PerformanceObserver.supportedEntryTypes.includes('longtask'));
  expect(supported, 'this browser reports longtask entries (use Chromium for long-task budgets)').toBe(true);
  await page.evaluate(() => {
    const w = window as unknown as { __perfLt: number[]; __perfLtObs?: PerformanceObserver };
    w.__perfLt = [];
    w.__perfLtObs?.disconnect();
    w.__perfLtObs = new PerformanceObserver((list) => {
      for (const e of list.getEntries()) w.__perfLt.push(e.duration);
    });
    w.__perfLtObs.observe({ type: 'longtask', buffered: false });
  });
}

/** Durations (ms) of the long tasks seen since `watchLongTasks`. */
export async function longTasks(page: Page): Promise<number[]> {
  return page.evaluate(() => (window as unknown as { __perfLt?: number[] }).__perfLt ?? []);
}

// ── frame deltas (any engine; WebKit for frame budgets) ──────────────────────

/** Record rAF-to-rAF deltas while `action` runs (plus `settleFrames` after
 *  it), and return their distribution. Deltas over the frame are the jank. */
export async function frameDeltas(page: Page, action: () => Promise<unknown>, settleFrames = 10): Promise<Dist> {
  await page.evaluate(() => {
    const w = window as unknown as { __perfFd: number[]; __perfFdOn: boolean };
    w.__perfFd = [];
    w.__perfFdOn = true;
    let last = performance.now();
    const loop = (t: number): void => {
      if (!w.__perfFdOn) return;
      w.__perfFd.push(t - last);
      last = t;
      requestAnimationFrame(loop);
    };
    requestAnimationFrame((t) => {
      last = t;
      requestAnimationFrame(loop);
    });
  });
  await action();
  const deltas = await page.evaluate(async (n) => {
    for (let i = 0; i < n; i++) await new Promise((r) => requestAnimationFrame(() => r(null)));
    const w = window as unknown as { __perfFd: number[]; __perfFdOn: boolean };
    w.__perfFdOn = false;
    return w.__perfFd;
  }, settleFrames);
  return dist(deltas);
}

// ── DOM ───────────────────────────────────────────────────────────────────────

/** Elements matching `sel` right now (a DOM-size budget). */
export async function domCount(page: Page, sel = '*'): Promise<number> {
  return page.evaluate((s) => document.querySelectorAll(s).length, sel);
}

/** Count `style` attribute writes on <html> from now on (full-document
 *  recalc triggers, SF-02). Returns a reader. */
export async function watchRootStyleWrites(page: Page): Promise<() => Promise<number>> {
  await page.evaluate(() => {
    const w = window as unknown as { __perfRootStyle: number; __perfRootObs?: MutationObserver };
    w.__perfRootStyle = 0;
    w.__perfRootObs?.disconnect();
    w.__perfRootObs = new MutationObserver((recs) => (w.__perfRootStyle += recs.length));
    w.__perfRootObs.observe(document.documentElement, { attributes: true, attributeFilter: ['style'] });
    w.__perfRootObs.observe(document.body, { attributes: true, attributeFilter: ['style'] });
  });
  return () => page.evaluate(() => (window as unknown as { __perfRootStyle: number }).__perfRootStyle);
}

// ── requests ─────────────────────────────────────────────────────────────────

export interface RequestStats {
  count: number;
  /** Request body bytes (what the page sent). */
  bytesSent: number;
  /** Largest single request body. */
  maxBody: number;
  /** Response body bytes that finished (from `sizes()`, best effort). */
  bytesReceived: number;
  /** Max requests in flight at once. */
  maxInFlight: number;
  /** Method + path of every matched request, in order. */
  paths: string[];
}

export interface RequestLog {
  stats(): RequestStats;
  /** Reset the counters (the listener keeps running). */
  reset(): void;
  stop(): void;
}

/** Log requests whose URL matches `pattern`: count, bytes, max concurrency. */
export function requestLog(page: Page, pattern: RegExp): RequestLog {
  let inFlight = new Set<Request>();
  let s: RequestStats = { count: 0, bytesSent: 0, maxBody: 0, bytesReceived: 0, maxInFlight: 0, paths: [] };
  const onReq = (r: Request): void => {
    if (!pattern.test(r.url())) return;
    const body = r.postDataBuffer()?.byteLength ?? 0;
    s.count++;
    s.bytesSent += body;
    s.maxBody = Math.max(s.maxBody, body);
    const u = new URL(r.url());
    s.paths.push(`${r.method()} ${u.pathname}${u.search}`);
    inFlight.add(r);
    s.maxInFlight = Math.max(s.maxInFlight, inFlight.size);
  };
  const onDone = (r: Request): void => {
    if (!inFlight.delete(r)) return;
    const stats = s;
    void r
      .sizes()
      .then((z) => (stats.bytesReceived += z.responseBodySize))
      .catch(() => {});
  };
  page.on('request', onReq);
  page.on('requestfinished', onDone);
  page.on('requestfailed', onDone);
  return {
    stats: () => ({ ...s, paths: [...s.paths] }),
    reset: () => {
      inFlight = new Set();
      s = { count: 0, bytesSent: 0, maxBody: 0, bytesReceived: 0, maxInFlight: 0, paths: [] };
    },
    stop: () => {
      page.off('request', onReq);
      page.off('requestfinished', onDone);
      page.off('requestfailed', onDone);
    },
  };
}

// ── visibility ────────────────────────────────────────────────────────────────

/** Pretend the document is hidden (or visible again) and fire
 *  `visibilitychange`, the way pollWhileVisible / the clock listen for it. */
export async function setDocumentHidden(page: Page, hidden: boolean): Promise<void> {
  await page.evaluate((h) => {
    Object.defineProperty(document, 'visibilityState', { configurable: true, get: () => (h ? 'hidden' : 'visible') });
    Object.defineProperty(document, 'hidden', { configurable: true, get: () => h });
    document.dispatchEvent(new Event('visibilitychange'));
  }, hidden);
}

// ── heap (Chromium, CDP) ─────────────────────────────────────────────────────

/** JS heap bytes after a forced GC (Chromium only — asserts CDP is there). */
export async function heapAfterGC(page: Page): Promise<number> {
  let cdp;
  try {
    cdp = await page.context().newCDPSession(page);
  } catch {
    expect(false, 'heapAfterGC needs Chromium (CDP)').toBe(true);
    return 0;
  }
  try {
    await cdp.send('HeapProfiler.collectGarbage');
    const { usedSize } = await cdp.send('Runtime.getHeapUsage');
    return usedSize;
  } finally {
    await cdp.detach().catch(() => {});
  }
}
