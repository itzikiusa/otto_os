// Shared e2e perf probes (GAPS_TO_9_5 §0 "E"). One copy of the measuring code
// every perf spec needs, so budgets read the same way everywhere.
//
// Rules for a perf spec (see desktop-db-editor-perf.spec.ts):
// - `test.skip(!isDesktopProject(project))`: it runs in `desktop-browser`
//   (Chromium) AND `desktop-webkit` (WebKit, closest to WKWebView).
// - A WebKit-only budget (keystroke/frame/scroll timings calibrated on WebKit)
//   uses `test.skip(!isWebkitProject(project))`; a Chromium-only probe
//   (`watchLongTasks`, `heapAfterGC`) `test.skip(browserName !== 'chromium')`
//   — both assert support, so a check is never vacuous on the wrong engine.
// - Timings end AFTER the frame's paint (`keyFrameCosts`, `frameWork`), not at
//   a microtask: WKWebView's cost is mostly style/layout/paint (r3-10-02).
// - Budget DOM/request COUNTS first, timings second: counts don't flake.

/** The desktop projects every perf spec runs in. */
export const DESKTOP_PROJECTS = ['desktop-browser', 'desktop-webkit'] as const;

export function isDesktopProject(name: string): boolean {
  return (DESKTOP_PROJECTS as readonly string[]).includes(name);
}

/** The WebKit desktop project (frame/keystroke budgets are WebKit budgets). */
export function isWebkitProject(name: string): boolean {
  return name === 'desktop-webkit';
}

/** Collect the app's fatal UI errors (main.ts: an effect loop or crash that
 *  reloads the page). A perf number measured across such a reload is noise,
 *  so gates assert the returned list stays empty. */
export function watchFatalUiErrors(page: Page): string[] {
  const fatal: string[] = [];
  page.on('console', (m) => {
    const t = m.text();
    if (t.includes('fatal UI error')) fatal.push(t.slice(0, 200));
  });
  return fatal;
}

/** A timing budget scaled by `OTTO_PERF_BUDGET_SCALE` (default 1): a shared
 *  CI runner is slower than a dev Mac, so CI widens TIMINGS only — DOM and
 *  request counts are never scaled. */
export function budgetMs(ms: number): number {
  const k = Number(process.env.OTTO_PERF_BUDGET_SCALE ?? '1');
  return ms * (Number.isFinite(k) && k > 0 ? k : 1);
}

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

// ── work up to the end of the painted frame (any engine) ─────────────────────
//
// A sample that ends at a microtask (or right after a forced layout) misses
// the paint, and on WebKit style/layout/PAINT are most of the cost. These end
// on a MessageChannel task posted from a requestAnimationFrame callback: it
// runs after that frame's style, layout and paint. The idle wait for vsync is
// NOT counted — the sample is script + the frame's rendering work, so it is
// comparable across refresh rates.

/** Record, per keystroke from now on: script from keydown (capture) to the
 *  microtask after its handlers, PLUS the rendering work of the next frame
 *  (rAF callback → post-paint task). Read with `keyFrameCosts`. */
export async function watchKeyFrameCosts(page: Page): Promise<void> {
  await page.evaluate(() => {
    const w = window as unknown as { __kf: number[]; __kfOn?: boolean };
    w.__kf = [];
    if (w.__kfOn) return;
    w.__kfOn = true;
    let t0 = 0;
    let script = 0;
    // keydown → input run in ONE task; a microtask queued by the bubble-phase
    // `input` listener (or bubble keydown, for a key an editor consumes) runs
    // after every handler and the reactive flush they queued.
    const scriptEnd = () => queueMicrotask(() => (script = performance.now() - t0));
    window.addEventListener(
      'keydown',
      () => {
        t0 = performance.now();
        script = 0;
        requestAnimationFrame(() => {
          const r0 = performance.now();
          const ch = new MessageChannel();
          ch.port1.onmessage = () => {
            w.__kf.push(script + (performance.now() - r0));
            ch.port1.close();
          };
          ch.port2.postMessage(null);
        });
      },
      true,
    );
    window.addEventListener('input', scriptEnd);
    window.addEventListener('keydown', (e) => {
      if (e.defaultPrevented) scriptEnd();
    });
  });
}

export async function keyFrameCosts(page: Page): Promise<number[]> {
  return page.evaluate(() => (window as unknown as { __kf?: number[] }).__kf ?? []);
}

/** Scroll `selector` by `dy` inside a rAF callback, `steps` times, timing
 *  each step to the end of that frame's paint: the scroll handler, its
 *  reactive flush, style, layout and paint — no vsync wait. */
export async function scrollFrameWork(page: Page, selector: string, dy: number, steps: number): Promise<number[]> {
  return page.evaluate(
    async ({ selector, dy, steps }) => {
      const el = document.querySelector(selector) as HTMLElement;
      const out: number[] = [];
      for (let i = 0; i < steps; i++) {
        out.push(
          await new Promise<number>((resolve) =>
            requestAnimationFrame(() => {
              const t0 = performance.now();
              el.scrollTop += dy;
              el.dispatchEvent(new Event('scroll'));
              const ch = new MessageChannel();
              ch.port1.onmessage = () => {
                ch.port1.close();
                resolve(performance.now() - t0);
              };
              ch.port2.postMessage(null);
            }),
          ),
        );
      }
      return out;
    },
    { selector, dy, steps },
  );
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
