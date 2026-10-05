import { test, expect, type Browser, type BrowserContext, type Page } from '@playwright/test';
import { join } from 'node:path';
import { apiCtx, daemonInfo, seedWorkspace } from './seed';

// ─────────────────────────────────────────────────────────────────────────────
// Transport: the webview↔daemon link (TRANSPORT_PLAN stage 0 + its stage 1/2
// pass criteria). The UI reaches the daemon over plain HTTP/1.1 and the engine
// opens at most ~6 sockets per HOST, shared by every page of one network
// session (every Otto window). This spec measures, in Chromium AND WebKit
// (≈ Tauri's WKWebView), from a page on the UI origin — so every call is
// cross-origin + `Authorization`, i.e. preflighted, exactly like the app:
//
//  M1  `/health` latency while 12 long-held requests (`/sessions/{id}/wait`,
//      holds HOLD_S) + 20 lists saturate the SAME host (the pre-fix symptom:
//      ≈ the hold time), and — once the daemon advertises `alt_loopback_base`
//      — while the same load sits on the `[::1]` alias lane (must stay fast);
//  M2  do WebSockets count against the 6? `/health` with 6 and 8 open
//      `/ws/term/{id}` sockets and no REST load;
//  M3  requests (and preflights, Chromium via CDP) per 30 s on an idle Home;
//  M4  two pages in one context (a stand-in for main + tray windows, which
//      share one network process): saturate from A, time `/health` from B;
//  M5  the tray (`#/tray`) idle for 45 s — it used to poll 4 endpoints / 20 s.
//
// Every number is printed as `[transport] …` and attached to the report.
// `OTTO_TRANSPORT_BASELINE=1` records without asserting (the stage-0 run
// against the pre-fix tree); otherwise the stage 1/2 budgets below apply.
// ─────────────────────────────────────────────────────────────────────────────

const BASELINE = process.env.OTTO_TRANSPORT_BASELINE === '1';
/** Seconds each saturating `/wait` holds its socket. */
const HOLD_S = 6;
/** A probe slower than this is "stalled behind the pool". */
const STALL_MS = 1_500;
const SLOT = process.env.OTTO_E2E_SLOT ?? '0';
const STATE = join(process.cwd(), 'e2e', `.auth-${SLOT}`, 'state.json');

let wsId = '';
const sids: string[] = [];
const results: Record<string, unknown> = {};

function record(key: string, value: unknown): void {
  results[key] = value;
  // eslint-disable-next-line no-console
  console.log(`[transport] ${key}: ${JSON.stringify(value)}`);
}

test.describe.configure({ mode: 'serial' });

test.beforeAll(async () => {
  test.setTimeout(60_000);
  const { ctx, base } = await apiCtx();
  wsId = await seedWorkspace(ctx, base);
  for (let i = 0; i < 3; i++) {
    const r = await ctx.post(`${base}/api/v1/workspaces/${wsId}/sessions`, {
      data: { kind: 'agent', provider: 'shell', title: `transport-${i}`, cwd: '/tmp', meta: { origin: 'e2e' } },
    });
    if (!r.ok()) throw new Error(`seed session → ${r.status()} ${await r.text()}`);
    sids.push(((await r.json()) as { id: string }).id);
  }
  await ctx.dispose();
});

test.beforeEach(async ({}, info) => {
  test.skip(info.project.name !== 'desktop-browser', 'desktop-browser project only');
});

test.afterAll(async ({}, info) => {
  if (info.project.name !== 'desktop-browser') return;
  await info.attach('transport-results.json', {
    body: JSON.stringify(results, null, 2),
    contentType: 'application/json',
  });
});

type Engine = 'chromium' | 'webkit';

/** A context on the UI origin with the e2e token, in the requested engine. */
async function openContext(
  engine: Engine,
  browser: Browser,
  launchWebkit: () => Promise<Browser>,
  baseURL: string,
): Promise<{ ctx: BrowserContext; close: () => Promise<void> }> {
  const b = engine === 'chromium' ? browser : await launchWebkit();
  const ctx = await b.newContext({ storageState: STATE, baseURL, serviceWorkers: 'block' });
  return {
    ctx,
    close: async () => {
      await ctx.close().catch(() => {});
      if (b !== browser) await b.close().catch(() => {});
    },
  };
}

/** A light document on the UI origin (a static asset — the app never boots). */
async function originPage(ctx: BrowserContext): Promise<Page> {
  const page = await ctx.newPage();
  await page.goto('/favicon.svg');
  return page;
}

interface ProbeArgs {
  base: string;
  token: string;
}

/** Time one `/health` (preflight pre-warmed, cache bypassed). Resolves to the
 *  latency in ms, or -1 when it did not finish within `limitMs`. */
async function probe(page: Page, a: ProbeArgs, limitMs = 20_000): Promise<number> {
  return page.evaluate(
    async ({ base, token, limitMs }) => {
      const ctl = new AbortController();
      const t = setTimeout(() => ctl.abort(), limitMs);
      const t0 = performance.now();
      try {
        await fetch(`${base}/api/v1/health`, {
          headers: { Authorization: `Bearer ${token}` },
          cache: 'no-store',
          signal: ctl.signal,
        });
        return Math.round(performance.now() - t0);
      } catch {
        return -1;
      } finally {
        clearTimeout(t);
      }
    },
    { ...a, limitMs },
  );
}

/** Warm `/health`'s preflight so a probe times only the GET. */
async function warm(page: Page, a: ProbeArgs): Promise<number[]> {
  const out: number[] = [];
  for (let i = 0; i < 5; i++) out.push(await probe(page, a));
  return out.sort((x, y) => x - y);
}

/** Fire the saturator from `page` against `satBase` (not awaited): 12 held
 *  `/wait`s + 20 lists, each a distinct URL (no cache-lock coalescing). */
async function saturate(page: Page, satBase: string, token: string, sid: string, tag: string): Promise<void> {
  await page.evaluate(
    ({ satBase, token, sid, hold, tag }) => {
      const h = { Authorization: `Bearer ${token}` };
      const all: Promise<unknown>[] = [];
      for (let i = 0; i < 12; i++) {
        all.push(
          fetch(`${satBase}/api/v1/sessions/${sid}/wait?status=exited&timeout_secs=${hold}&n=${tag}${i}`, {
            headers: h,
            cache: 'no-store',
          }).catch(() => null),
        );
      }
      for (let i = 0; i < 20; i++) {
        all.push(fetch(`${satBase}/api/v1/workspaces?n=${tag}${i}`, { headers: h, cache: 'no-store' }).catch(() => null));
      }
      (window as unknown as { __sat: Promise<unknown> }).__sat = Promise.allSettled(all);
    },
    { satBase, token, sid, hold: HOLD_S, tag },
  );
  // Let the held requests (and their preflights) take the sockets.
  await page.waitForTimeout(700);
}

async function drain(page: Page): Promise<void> {
  await page.evaluate(() => (window as unknown as { __sat?: Promise<unknown> }).__sat);
}

/** The daemon's advertised second loopback host, or null (pre-fix / no IPv6). */
async function aliasBase(page: Page, a: ProbeArgs): Promise<string | null> {
  return page.evaluate(async ({ base, token }) => {
    try {
      const r = await fetch(`${base}/api/v1/meta`, { headers: { Authorization: `Bearer ${token}` } });
      const m = (await r.json()) as { alt_loopback_base?: string | null };
      return m.alt_loopback_base ?? null;
    } catch {
      return null;
    }
  }, a);
}

for (const engine of ['chromium', 'webkit'] as const) {
  test(`${engine}: M1/M2/M4 — interactive latency under a saturated pool`, async ({ browser, playwright, baseURL }) => {
    test.setTimeout(150_000);
    const { base, token } = daemonInfo();
    const a = { base, token };
    const { ctx, close } = await openContext(engine, browser, () => playwright.webkit.launch(), baseURL!);
    try {
      const page = await originPage(ctx);
      const idle = await warm(page, a);
      record(`${engine}.health_idle_ms`, idle);

      // M1 — same host.
      await saturate(page, base, token, sids[0], 'm1');
      const m1 = await probe(page, a);
      await drain(page);
      record(`${engine}.M1_same_host_saturated_ms`, m1);

      // M1 — saturation on the alias lane, probe on the interactive host.
      const alias = await aliasBase(page, a);
      record(`${engine}.alt_loopback_base`, alias);
      let m1Alias: number | null = null;
      if (alias) {
        await probe(page, { base: alias, token }); // warm the alias preflight
        await saturate(page, alias, token, sids[0], 'm1a');
        m1Alias = await probe(page, a);
        await drain(page);
        record(`${engine}.M1_alias_saturated_ms`, m1Alias);
      }

      // M2 — WebSockets vs the per-host pool (no REST load).
      const wsResult: Record<string, number> = {};
      for (const n of [6, 8]) {
        const opened = await page.evaluate(
          async ({ base, token, sids, n }) => {
            const url = base.replace(/^http/, 'ws');
            const socks: WebSocket[] = [];
            const opens = Array.from({ length: n }, (_, i) => {
              const s = new WebSocket(`${url}/ws/term/${sids[i % sids.length]}`, ['otto-bearer', token]);
              socks.push(s);
              return new Promise<boolean>((res) => {
                s.onopen = () => res(true);
                s.onerror = () => res(false);
                setTimeout(() => res(false), 5_000);
              });
            });
            (window as unknown as { __socks: WebSocket[] }).__socks = socks;
            return (await Promise.all(opens)).filter(Boolean).length;
          },
          { base, token, sids, n },
        );
        const dt = await probe(page, a, 5_000);
        await page.evaluate(() => {
          for (const s of (window as unknown as { __socks: WebSocket[] }).__socks) s.close();
        });
        await page.waitForTimeout(300);
        wsResult[`open_${opened}_of_${n}_health_ms`] = dt;
      }
      record(`${engine}.M2_ws_term`, wsResult);

      // M4 — two pages, one context (main + tray share a network process).
      const pageB = await originPage(ctx);
      await warm(pageB, a);
      await saturate(page, base, token, sids[1], 'm4');
      const m4 = await probe(pageB, a);
      await drain(page);
      record(`${engine}.M4_cross_page_same_host_ms`, m4);
      let m4Alias: number | null = null;
      if (alias) {
        await saturate(page, alias, token, sids[1], 'm4a');
        m4Alias = await probe(pageB, a);
        await drain(page);
        record(`${engine}.M4_cross_page_alias_ms`, m4Alias);
      }

      if (!BASELINE) {
        expect(idle[2], 'idle /health p50').toBeLessThan(500);
        // Stage 1 pass criteria: background/long load on the alias lane never
        // blocks an interactive call — in this page or another.
        if (alias) {
          expect(m1Alias, 'M1 with the saturator on the alias lane').toBeGreaterThanOrEqual(0);
          expect(m1Alias!, 'M1 with the saturator on the alias lane').toBeLessThan(STALL_MS);
          expect(m4Alias!, 'M4 cross-page with the saturator on the alias lane').toBeGreaterThanOrEqual(0);
          expect(m4Alias!, 'M4 cross-page with the saturator on the alias lane').toBeLessThan(STALL_MS);
        } else {
          test.info().annotations.push({ type: 'note', description: 'daemon advertised no alt_loopback_base (IPv6 off?)' });
        }
      }
    } finally {
      await close();
    }
  });

  test(`${engine}: M3 — requests per 30 s on an idle Home`, async ({ browser, playwright, baseURL }) => {
    test.setTimeout(120_000);
    const { base } = daemonInfo();
    const { ctx, close } = await openContext(engine, browser, () => playwright.webkit.launch(), baseURL!);
    try {
      await ctx.addInitScript((id) => {
        localStorage.setItem('otto_workspace', id as string);
        localStorage.setItem('otto_firstrun_dismissed', '1');
      }, wsId);
      const page = await ctx.newPage();
      const seen = { req: 0, options: 0, cdpOptions: 0, paths: {} as Record<string, number> };
      let counting = false;
      page.on('request', (r) => {
        if (!counting || !r.url().startsWith(base)) return;
        if (r.method() === 'OPTIONS') seen.options += 1;
        else {
          seen.req += 1;
          const p = new URL(r.url()).pathname.replace(/[0-9a-f-]{16,}/g, ':id');
          seen.paths[p] = (seen.paths[p] ?? 0) + 1;
        }
      });
      if (engine === 'chromium') {
        // Preflights are not page requests in Chromium — count them via CDP.
        const cdp = await ctx.newCDPSession(page);
        await cdp.send('Network.enable');
        cdp.on('Network.requestWillBeSent', (e) => {
          if (counting && e.request.url.startsWith(base) && (e.request.method === 'OPTIONS' || e.type === 'Preflight')) {
            seen.cdpOptions += 1;
          }
        });
      }
      await page.goto('/#/home');
      await expect(page.locator('.content').first()).toBeVisible({ timeout: 20_000 });
      await page.waitForTimeout(5_000); // boot burst settles
      counting = true;
      await page.waitForTimeout(30_000);
      counting = false;
      record(`${engine}.M3_home_idle_30s`, seen);
    } finally {
      await close();
    }
  });
}

test('chromium: M5 — the tray idles without polling', async ({ browser, baseURL }) => {
  test.setTimeout(120_000);
  const { base } = daemonInfo();
  const ctx = await browser.newContext({ storageState: STATE, baseURL, serviceWorkers: 'block' });
  try {
    const page = await ctx.newPage();
    const seen = { req: 0, paths: {} as Record<string, number>, ws: [] as string[] };
    let counting = false;
    page.on('request', (r) => {
      if (!counting || !r.url().startsWith(base) || r.method() === 'OPTIONS') return;
      seen.req += 1;
      const p = new URL(r.url()).pathname;
      seen.paths[p] = (seen.paths[p] ?? 0) + 1;
    });
    page.on('websocket', (w) => seen.ws.push(new URL(w.url()).pathname));
    await page.goto('/#/tray');
    await expect(page.locator('.tray')).toBeVisible({ timeout: 20_000 });
    await page.waitForTimeout(4_000); // first load settles
    counting = true;
    await page.waitForTimeout(45_000);
    counting = false;
    record('chromium.M5_tray_idle_45s', seen);
    if (!BASELINE) {
      // Stage 2: the tray is event-fed (filtered /ws/events) with a 5 min
      // safety net — no request while nothing changes.
      expect(seen.ws, 'tray opens the event socket').toContain('/ws/events');
      const telemetryRequests = Object.entries(seen.paths)
        .filter(([path]) => path.startsWith('/api/v1/telemetry/'))
        .reduce((sum, [, count]) => sum + count, 0);
      expect(seen.req - telemetryRequests, 'tray business-data requests while idle').toBeLessThanOrEqual(1);
      // Opt-in adds the settled boot batch and the 30-second consent check.
      // Keep its budget separate so it cannot hide renewed session polling.
      expect(telemetryRequests, 'bounded opt-in telemetry traffic')
        .toBeLessThanOrEqual(process.env.OTTO_E2E_TELEMETRY === '1' ? 3 : 0);
    }
  } finally {
    await ctx.close().catch(() => {});
  }
});
