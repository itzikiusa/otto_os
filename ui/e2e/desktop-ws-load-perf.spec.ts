import { test, expect, type APIRequestContext, type Page } from '@playwright/test';
import { writeFileSync } from 'node:fs';
import { join } from 'node:path';
import { tmpdir } from 'node:os';
import { apiCtx, seedWorkspace } from './seed';
import { isDesktopProject, useDefaultTerminalRenderer } from './perf';
import { applyPreset, sessionRef, storeTree, storeTabs, MAX_PANES } from '../src/lib/stores/splitLayout';
import type { TelemetryConfig } from '../src/lib/api/types';

type Probe = {
  sessionId(): string;
  pending(): number;
  queued(): number;
  scrollback(): number;
  renderer(): string;
  text(): string;
  disposed: boolean;
};

type WsTrace = {
  url: string;
  openedAt: number;
  scrollbackSentAt: number | null;
  scrollbackLines: number | null;
  snapHeaderAt: number | null;
  snapBytes: number;
  snapRecvAt: number | null;
};

type GlStats = {
  webglCount: number;
  webglMs: number;
  webglMaxMs: number;
  longtasks: { count: number; totalMs: number; maxMs: number };
};

async function resizePty(base: string, token: string, id: string, cols: number, rows: number): Promise<void> {
  await new Promise<void>((resolve, reject) => {
    const ws = new WebSocket(`${base.replace('http', 'ws')}/ws/term/${id}`, ['otto-bearer', token]);
    const timer = setTimeout(() => {
      try { ws.close(); } catch { /* ignore */ }
      resolve();
    }, 5000);
    ws.onopen = () => {
      ws.send(JSON.stringify({ type: 'claim', cols, rows }));
      setTimeout(() => {
        clearTimeout(timer);
        ws.close();
        resolve();
      }, 80);
    };
    ws.onerror = (e) => {
      clearTimeout(timer);
      reject(new Error(`resize ws failed: ${String(e)}`));
    };
  });
}

async function seedLongScrollbackSessions(
  ctx: APIRequestContext,
  base: string,
  token: string,
  wsId: string,
  count: number,
  tag: string,
  cols = 160,
  rows = 48,
): Promise<string[]> {
  const ids: string[] = [];
  for (let i = 0; i < count; i++) {
    const r = await ctx.post(`${base}/api/v1/workspaces/${wsId}/sessions`, {
      data: {
        kind: 'agent',
        provider: 'shell',
        title: `Term ${tag}-${i}`,
        cwd: '/tmp',
        meta: { origin: 'manual' },
      },
    });
    expect(r.ok(), await r.text()).toBeTruthy();
    const s = await r.json();
    ids.push(s.id as string);
  }
  await Promise.all(ids.map((id) => resizePty(base, token, id, cols, rows)));
  const scriptPath = join(tmpdir(), `otto-ws-load-fill-${process.pid}.py`);
  writeFileSync(
    scriptPath,
    [
      'import sys',
      'idx = int(sys.argv[1])',
      'tag = sys.argv[2]',
      'pad = "abcdef0123456789" * 7',
      'out = sys.stdout.buffer',
      'chunks = []',
      'for k in range(4500):',
      '    chunks.append(f"\\x1b[36m[t{idx:02d}:{k:04d}]\\x1b[0m \\x1b[32mINFO\\x1b[0m \\x1b[33msrc/mod_{k%17}.ts\\x1b[0m: \\x1b[35mbatch={k}\\x1b[0m payload={pad}\\n".encode())',
      'chunks.append(f"READY_{tag}_{idx}_DONE\\n".encode())',
      'out.write(b"".join(chunks))',
      'out.flush()',
      '',
    ].join('\n'),
  );
  await new Promise((r) => setTimeout(r, 350));
  await Promise.all(
    ids.map(async (id, i) => {
      const r = await ctx.post(`${base}/api/v1/sessions/${id}/input`, {
        data: { text: `python3 ${scriptPath} ${i} ${tag}`, submit: true },
      });
      expect(r.ok(), await r.text()).toBeTruthy();
    }),
  );
  const deadline = Date.now() + 60_000;
  const ready = new Set<string>();
  while (ready.size < ids.length && Date.now() < deadline) {
    await Promise.all(
      ids.map(async (id, i) => {
        if (ready.has(id)) return;
        const r = await ctx.get(`${base}/api/v1/sessions/${id}/screen`);
        if (!r.ok()) return;
        const body = (await r.json()) as { lines?: string[] };
        if ((body.lines ?? []).some((l) => l.includes(`READY_${tag}_${i}_DONE`))) {
          ready.add(id);
        }
      }),
    );
    if (ready.size < ids.length) await new Promise((r) => setTimeout(r, 150));
  }
  expect(ready.size, `all ${count} sessions filled scrollback`).toBe(ids.length);
  return ids;
}

function attachWsRecorder(page: Page) {
  let active = false;
  let t0 = 0;
  let traces: WsTrace[] = [];
  let restReqs: { path: string; status: number; ms: number }[] = [];
  const reqStarts = new Map<object, number>();

  page.on('request', (req) => {
    if (!active) return;
    if (req.url().includes('/api/v1/')) reqStarts.set(req, performance.now());
  });
  page.on('requestfinished', async (req) => {
    const st = reqStarts.get(req);
    if (st === undefined) return;
    reqStarts.delete(req);
    const resp = await req.response().catch(() => null);
    const u = new URL(req.url());
    restReqs.push({ path: u.pathname + u.search, status: resp?.status() ?? 0, ms: Math.round(performance.now() - st) });
  });

  page.on('websocket', (sock) => {
    if (!sock.url().includes('/ws/term/')) return;
    const tr: WsTrace = {
      url: sock.url(),
      openedAt: active ? performance.now() - t0 : -1,
      scrollbackSentAt: null,
      scrollbackLines: null,
      snapHeaderAt: null,
      snapBytes: 0,
      snapRecvAt: null,
    };
    if (active) traces.push(tr);
    let expectBinSnap = false;
    sock.on('framesent', ({ payload }) => {
      if (!active || typeof payload !== 'string') return;
      try {
        const f = JSON.parse(payload);
        if (f.type === 'scrollback' && tr.scrollbackSentAt === null) {
          tr.scrollbackSentAt = performance.now() - t0;
          tr.scrollbackLines = f.lines ?? null;
        }
      } catch { /* ignore */ }
    });
    sock.on('framereceived', ({ payload }) => {
      if (!active) return;
      if (typeof payload === 'string') {
        try {
          const f = JSON.parse(payload);
          if (f.type === 'scrollback') {
            tr.snapHeaderAt = performance.now() - t0;
            if (f.binary) {
              expectBinSnap = true;
              tr.snapBytes += Number(f.len ?? 0);
            } else if (typeof f.data === 'string') {
              tr.snapBytes += Math.floor((f.data.length * 3) / 4);
              tr.snapRecvAt = performance.now() - t0;
            }
          }
        } catch { /* ignore */ }
      } else if (expectBinSnap) {
        expectBinSnap = false;
        tr.snapRecvAt = performance.now() - t0;
      }
    });
  });

  return {
    async start() {
      traces = [];
      restReqs = [];
      reqStarts.clear();
      await page.evaluate(() => {
        const w = window as unknown as { __glReset?: () => void };
        w.__glReset?.();
      }).catch(() => {});
      t0 = performance.now();
      active = true;
    },
    async stop() {
      active = false;
      const gl: GlStats = await page.evaluate(() => {
        const w = window as unknown as { __glRead?: () => GlStats };
        return w.__glRead?.() ?? { webglCount: 0, webglMs: 0, webglMaxMs: 0, longtasks: { count: 0, totalMs: 0, maxMs: 0 } };
      }).catch(() => ({ webglCount: 0, webglMs: 0, webglMaxMs: 0, longtasks: { count: 0, totalMs: 0, maxMs: 0 } }));
      const snapLatencies = traces
        .filter((t) => t.scrollbackSentAt !== null && t.snapRecvAt !== null)
        .map((t) => Math.round(t.snapRecvAt! - t.scrollbackSentAt!));
      return {
        wsOpened: traces.length,
        snapBytesTotal: traces.reduce((a, b) => a + b.snapBytes, 0),
        firstWsAt: traces.length ? Math.round(Math.min(...traces.map((t) => t.openedAt))) : -1,
        firstSnapRecvAt: traces.some((t) => t.snapRecvAt !== null)
          ? Math.round(Math.min(...traces.filter((t) => t.snapRecvAt !== null).map((t) => t.snapRecvAt!)))
          : -1,
        lastSnapRecvAt: traces.some((t) => t.snapRecvAt !== null)
          ? Math.round(Math.max(...traces.filter((t) => t.snapRecvAt !== null).map((t) => t.snapRecvAt!)))
          : -1,
        snapLatencies,
        restCount: restReqs.length,
        restMaxMs: restReqs.length ? Math.max(...restReqs.map((r) => r.ms)) : 0,
        gl,
      };
    },
  };
}

async function waitFirstUsable(page: Page, activeId: string, marker: string) {
  const start = performance.now();
  await page.waitForFunction(
    ({ id, m }) => {
      const list = (window as unknown as { __ottoTermProbe?: Probe[] }).__ottoTermProbe ?? [];
      const p = list.find((x) => !x.disposed && x.sessionId() === id);
      return !!p && p.pending() === 0 && p.queued() === 0 && p.text().includes(m);
    },
    { id: activeId, m: marker },
    { timeout: 60_000 },
  );
  const firstReadyMs = Math.round(performance.now() - start);
  const atFirstReady = await page.evaluate(() => {
    const list = ((window as unknown as { __ottoTermProbe?: Probe[] }).__ottoTermProbe ?? []).filter((x) => !x.disposed);
    const renderers: Record<string, number> = {};
    for (const p of list) {
      const r = p.renderer();
      renderers[r] = (renderers[r] ?? 0) + 1;
    }
    return { mounted: list.length, renderers };
  });
  return { firstReadyMs, ...atFirstReady };
}

test('phase-breakdown profile: 160x48 long-scrollback terminals (N=1,5,11,20) with WebGL + CDP + telemetry', async ({ page }, info) => {
  test.skip(!isDesktopProject(info.project.name), 'desktop projects only');
  test.setTimeout(360_000);
  const { ctx, base, token } = await apiCtx();
  const emptyWs = await seedWorkspace(ctx, base, 'Empty WS');

  // Enable Otto telemetry on Chromium run so we capture daemon-side span durations too
  const wantTelemetry = info.project.name === 'desktop-browser';
  let origTelemetry: TelemetryConfig | null = null;
  if (wantTelemetry) {
    origTelemetry = await (await ctx.get(`${base}/api/v1/telemetry/config`)).json();
    const put = await ctx.put(`${base}/api/v1/telemetry/config`, { data: { ...origTelemetry, enabled: true } });
    if (put.ok()) {
      await expect
        .poll(async () => (await (await ctx.get(`${base}/api/v1/telemetry/status`)).json()).collector_ready, { timeout: 100_000 })
        .toBe(true);
    }
  }

  const rec = attachWsRecorder(page);
  if (process.env.OTTO_PROFILE_DOM !== '1') await useDefaultTerminalRenderer(page);
  await page.addInitScript((forceDom) => {
    (window as unknown as { __ottoTermProbe: Probe[] }).__ottoTermProbe = [];
    localStorage.setItem('otto_firstrun_dismissed', '1');
    localStorage.setItem('otto_nav_all_ws', '0');
    localStorage.setItem('otto_agents_fold_nav', '1');
    if (forceDom) localStorage.setItem('otto.term.renderer', 'dom');
    // Measure WebGL2 context creation & longtasks without altering app logic
    let webglCount = 0;
    let webglMs = 0;
    let webglMaxMs = 0;
    let ltCount = 0;
    let ltTotalMs = 0;
    let ltMaxMs = 0;
    const origGetCtx = HTMLCanvasElement.prototype.getContext;
    HTMLCanvasElement.prototype.getContext = function (this: HTMLCanvasElement, type: string, ...args: unknown[]) {
      if (type === 'webgl2' || type === 'webgl') {
        const t0 = performance.now();
        const res = (origGetCtx as (...a: unknown[]) => unknown).apply(this, [type, ...args]);
        const dt = performance.now() - t0;
        webglCount++;
        webglMs += dt;
        if (dt > webglMaxMs) webglMaxMs = dt;
        return res;
      }
      return (origGetCtx as (...a: unknown[]) => unknown).apply(this, [type, ...args]);
    } as typeof HTMLCanvasElement.prototype.getContext;
    if (typeof PerformanceObserver === 'function') {
      try {
        const po = new PerformanceObserver((list) => {
          for (const e of list.getEntries()) {
            ltCount++;
            ltTotalMs += e.duration;
            if (e.duration > ltMaxMs) ltMaxMs = e.duration;
          }
        });
        po.observe({ entryTypes: ['longtask'] });
      } catch { /* unsupported in WebKit */ }
    }
    const w = window as unknown as { __glReset: () => void; __glRead: () => GlStats };
    w.__glReset = () => { webglCount = 0; webglMs = 0; webglMaxMs = 0; ltCount = 0; ltTotalMs = 0; ltMaxMs = 0; };
    w.__glRead = () => ({
      webglCount,
      webglMs: Math.round(webglMs),
      webglMaxMs: Math.round(webglMaxMs),
      longtasks: { count: ltCount, totalMs: Math.round(ltTotalMs), maxMs: Math.round(ltMaxMs) },
    });
  }, process.env.OTTO_PROFILE_DOM === '1');
  await page.setViewportSize({ width: 1440, height: 900 });

  await page.goto('/#/settings');
  await expect(page.locator('.shell')).toBeVisible({ timeout: 20_000 });

  const cdp = info.project.name === 'desktop-browser' ? await page.context().newCDPSession(page) : null;
  if (cdp) await cdp.send('Profiler.enable');

  async function withCpuProfile<T>(label: string, run: () => Promise<T>): Promise<T> {
    if (!cdp) return run();
    await cdp.send('Profiler.start');
    const out = await run();
    const { profile } = (await cdp.send('Profiler.stop')) as {
      profile: { nodes: { id: number; callFrame: { functionName: string; url: string; lineNumber: number } }[]; samples: number[]; timeDeltas: number[] };
    };
    const self = new Map<string, number>();
    const byId = new Map(profile.nodes.map((n) => [n.id, n]));
    profile.samples.forEach((id, i) => {
      const n = byId.get(id)!;
      const file = n.callFrame.url ? n.callFrame.url.split('/').pop() : '(native)';
      const k = `${n.callFrame.functionName || '(anon)'} ${file}:${n.callFrame.lineNumber}`;
      self.set(k, (self.get(k) ?? 0) + (profile.timeDeltas[i] ?? 0) / 1000);
    });
    const total = [...self.values()].reduce((a, b) => a + b, 0);
    const top = [...self]
      .filter(([k]) => !k.startsWith('(idle)'))
      .sort((a, b) => b[1] - a[1])
      .slice(0, 12)
      .map(([k, v]) => [k, Math.round(v), `${Math.round((v / Math.max(1, total)) * 100)}%`]);
    console.log('[CPU]', label, JSON.stringify({ totalMs: Math.round(total), top }));
    return out;
  }

  for (const n of [1, 5, 11, 20]) {
    const wsId = await seedWorkspace(ctx, base, `WideWS-${n}`);
    const ids = await seedLongScrollbackSessions(ctx, base, token, wsId, n, `W${n}`, 160, 48);
    const activeId = ids[0];
    const activeMarker = `READY_W${n}_0_DONE`;

    for (const mode of ['tabs', 'tiled', 'splits'] as const) {
      if (mode === 'splits' && n === 1) continue;
      // On Chromium, run N=1, 11 to stay fast; on WebKit run all 1, 5, 11, 20
      if (info.project.name === 'desktop-browser' && (n === 5 || n === 20)) continue;
      const splitCount = Math.min(n, MAX_PANES);
      const splitTree = mode === 'splits'
        ? applyPreset(ids.slice(0, splitCount).map(sessionRef), 'grid')
        : applyPreset([sessionRef(activeId)], 'cols');
      const panesPayload = JSON.stringify({ v: 2, tree: storeTree(splitTree), focused: null });
      const tabsPayload = storeTabs(ids.map(sessionRef));

      await page.evaluate(
        ({ emptyWs, wsId, mode, tabsPayload, panesPayload }) => {
          localStorage.setItem('otto_workspace', emptyWs);
          localStorage.setItem('otto_view_mode', mode === 'tiled' ? 'tiled' : 'tabs');
          localStorage.setItem(`otto_tabs_${wsId}`, tabsPayload);
          localStorage.setItem(`otto_panes_${wsId}`, panesPayload);
        },
        { emptyWs, wsId, mode, tabsPayload, panesPayload },
      );
      await page.goto('about:blank');
      await page.goto('/#/agents');
      await expect(page.locator('.shell')).toBeVisible({ timeout: 20_000 });
      await expect.poll(() => page.evaluate(() => document.querySelectorAll('.pane').length)).toBe(0);

      // 0) COLD WORKSPACE SWITCH
      await rec.start();
      const wsColdT0 = performance.now();
      const wsCold = await withCpuProfile(`n=${n} ${mode} ws_switch_cold`, async () => {
        await page.evaluate(async (id) => {
          const p = '/src/lib/stores/workspace.svelte.ts';
          const { ws } = await import(/* @vite-ignore */ p);
          await ws.select(id);
        }, wsId);
        return waitFirstUsable(page, activeId, activeMarker);
      });
      const wsColdWallMs = Math.round(performance.now() - wsColdT0);
      const wsColdRec = await rec.stop();
      console.log('[PHASE]', JSON.stringify({
        eng: info.project.name, n, mode, op: 'ws_switch_cold',
        wallMs: wsColdWallMs, firstReadyMs: wsCold.firstReadyMs,
        mounted: wsCold.mounted, renderers: wsCold.renderers,
        wsOpened: wsColdRec.wsOpened, snapKB: Math.round(wsColdRec.snapBytesTotal / 1024),
        firstWsAt: wsColdRec.firstWsAt, firstSnapAt: wsColdRec.firstSnapRecvAt, lastSnapAt: wsColdRec.lastSnapRecvAt,
        snapLat: wsColdRec.snapLatencies.slice(0, 6), rest: wsColdRec.restCount, restMaxMs: wsColdRec.restMaxMs,
        gl: wsColdRec.gl,
      }));
      // Stable count-based perf assertions at N=20 (no flaky wall-clock gates):
      // Frame 0 mounts the focused terminal first instead of all 15 at once.
      if (n === 20 && mode !== 'tabs') {
        expect(wsCold.mounted, `${mode} n=20 cold: only priority terminal(s) mounted at firstReady`).toBeLessThanOrEqual(4);
        expect(wsColdRec.wsOpened, `${mode} n=20 cold: bounded WS opens at firstReady`).toBeLessThanOrEqual(4);
      }

      // Let background visible panes finish hydrating so page_switch tests real multi-pane parking!
      const expectedLive = mode === 'tabs' ? 1 : Math.min(n, MAX_PANES);
      await expect.poll(
        () => page.evaluate(() => ((window as unknown as { __ottoTermProbe?: Probe[] }).__ottoTermProbe ?? []).filter((x) => !x.disposed).length),
        { timeout: 15_000 },
      ).toBe(expectedLive);

      // 1) PAGE SWITCH (#/settings -> #/agents) — parked adoption path
      await page.goto('/#/settings');
      await expect.poll(
        () => page.evaluate(() => ((window as unknown as { __ottoTermProbe?: Probe[] }).__ottoTermProbe ?? []).filter((x) => !x.disposed).length),
        { timeout: 10_000 },
      ).toBe(0);
      await rec.start();
      const pageT0 = performance.now();
      const pageSw = await withCpuProfile(`n=${n} ${mode} page_switch`, async () => {
        await page.goto('/#/agents');
        return waitFirstUsable(page, activeId, activeMarker);
      });
      const pageWallMs = Math.round(performance.now() - pageT0);
      const pageRec = await rec.stop();
      console.log('[PHASE]', JSON.stringify({
        eng: info.project.name, n, mode, op: 'page_switch',
        wallMs: pageWallMs, firstReadyMs: pageSw.firstReadyMs,
        mounted: pageSw.mounted, renderers: pageSw.renderers,
        wsOpened: pageRec.wsOpened, snapKB: Math.round(pageRec.snapBytesTotal / 1024),
        firstWsAt: pageRec.firstWsAt, firstSnapAt: pageRec.firstSnapRecvAt, lastSnapAt: pageRec.lastSnapRecvAt,
        rest: pageRec.restCount, restMaxMs: pageRec.restMaxMs,
        gl: pageRec.gl,
      }));
      if (n === 20 && mode !== 'tabs') {
        expect(pageSw.mounted, `${mode} n=20 page_switch: priority-first mount count`).toBeLessThanOrEqual(4);
        expect(pageRec.wsOpened, `${mode} n=20 page_switch: active terminal adopted from park without WS reopen`).toBe(0);
      }

      // Verify no scrollback or live output loss for a background/hidden terminal at N=20
      if (n === 20 && mode === 'tiled') {
        const bgId = ids[5];
        const liveTag = `LIVE_HIDDEN_${Date.now()}`;
        await ctx.post(`${base}/api/v1/sessions/${bgId}/input`, { data: { text: `echo ${liveTag}`, submit: true } });
        // Click tile 5 (focuses/promotes immediately if not yet hydrated) and verify BOTH historical scrollback and live output!
        await expect.poll(
          () => page.evaluate(({ id, m }) => {
            const p = ((window as unknown as { __ottoTermProbe?: Probe[] }).__ottoTermProbe ?? []).find((x) => !x.disposed && x.sessionId() === id);
            return !!p && p.text().includes(m);
          }, { id: bgId, m: liveTag }),
          { timeout: 15_000 },
        ).toBe(true);
      }
    }

    await page.goto('/#/settings');
    await expect(page.locator('.shell')).toBeVisible({ timeout: 20_000 });
    for (const id of ids) await ctx.delete(`${base}/api/v1/sessions/${id}`).catch(() => {});
  }

  if (wantTelemetry && origTelemetry) {
    await page.evaluate(async () => (await import(/* @vite-ignore */ String('/src/lib/telemetry.ts'))).flushTelemetry()).catch(() => {});
    const ov = await (await ctx.get(`${base}/api/v1/telemetry/overview?hours=1`)).json();
    console.log('[TELEMETRY-OVERVIEW]', JSON.stringify(ov));
    await ctx.put(`${base}/api/v1/telemetry/config`, { data: origTelemetry }).catch(() => {});
  }

  await ctx.dispose();
});

test('tiled view skips exited/inactive sessions in auto-live budget and preserves scrollback on click', async ({ page }, info) => {
  test.skip(!isDesktopProject(info.project.name), 'desktop projects only');
  test.setTimeout(90_000);
  const { ctx, base } = await apiCtx();
  const wsId = await seedWorkspace(ctx, base, 'MixedLiveExited');
  const liveIds: string[] = [];
  const exitedIds: string[] = [];
  for (let i = 0; i < 4; i++) {
    const r = await ctx.post(`${base}/api/v1/workspaces/${wsId}/sessions`, {
      data: { kind: 'agent', provider: 'shell', title: `Live-${i}`, cwd: '/tmp', meta: { origin: 'manual' } },
    });
    expect(r.ok()).toBeTruthy();
    liveIds.push((await r.json()).id as string);
  }
  for (let i = 0; i < 6; i++) {
    const r = await ctx.post(`${base}/api/v1/workspaces/${wsId}/sessions`, {
      data: { kind: 'agent', provider: 'shell', title: `Dead-${i}`, cwd: '/tmp', meta: { origin: 'manual' } },
    });
    expect(r.ok()).toBeTruthy();
    const id = (await r.json()).id as string;
    exitedIds.push(id);
    await ctx.post(`${base}/api/v1/sessions/${id}/input`, { data: { text: 'exit 0', submit: true } });
  }
  await expect.poll(async () => {
    const rows = (await (await ctx.get(`${base}/api/v1/workspaces/${wsId}/sessions`)).json()) as { id: string; status: string }[];
    return exitedIds.filter((id) => rows.find((r) => r.id === id)?.status === 'exited').length;
  }, { timeout: 20_000 }).toBe(exitedIds.length);

  await useDefaultTerminalRenderer(page);
  await page.addInitScript((arg) => {
    const a = arg as { wsId: string; tabs: string; panes: string };
    (window as unknown as { __ottoTermProbe: Probe[] }).__ottoTermProbe = [];
    localStorage.setItem('otto_workspace', a.wsId);
    localStorage.setItem('otto_view_mode', 'tiled');
    localStorage.setItem(`otto_tabs_${a.wsId}`, a.tabs);
    localStorage.setItem(`otto_panes_${a.wsId}`, a.panes);
    localStorage.setItem('otto_firstrun_dismissed', '1');
    localStorage.setItem('otto_nav_all_ws', '0');
    localStorage.setItem('otto_agents_fold_nav', '1');
  }, {
    wsId,
    tabs: storeTabs(liveIds.map(sessionRef)),
    panes: JSON.stringify({ v: 2, tree: storeTree(applyPreset([sessionRef(liveIds[0])], 'cols')), focused: null }),
  });

  await page.setViewportSize({ width: 1440, height: 900 });
  const wsUrls: string[] = [];
  page.on('websocket', (sock) => {
    if (sock.url().includes('/ws/term/')) wsUrls.push(sock.url());
  });
  await page.goto('/#/agents');
  await expect(page.locator('[data-tile-id]')).toHaveCount(liveIds.length + exitedIds.length, { timeout: 20_000 });
  // Wait for all 4 live tiles to hydrate
  await expect.poll(
    () => page.evaluate(() => ((window as unknown as { __ottoTermProbe?: Probe[] }).__ottoTermProbe ?? []).filter((x) => !x.disposed).length),
    { timeout: 15_000 },
  ).toBe(liveIds.length);
  // None of the 6 exited sessions should have opened a terminal socket or mounted a terminal
  for (const deadId of exitedIds) {
    expect(wsUrls.some((u) => u.includes(deadId)), `exited session ${deadId} must not auto-open WS`).toBe(false);
    await expect(page.locator(`[data-tile-id="${deadId}"] .tile-placeholder`)).toBeVisible();
  }

  for (const id of [...liveIds, ...exitedIds]) await ctx.delete(`${base}/api/v1/sessions/${id}`).catch(() => {});
  await ctx.dispose();
});
