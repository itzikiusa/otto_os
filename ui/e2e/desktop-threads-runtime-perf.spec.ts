import { test, expect, devices, type APIRequestContext, type Page, type CDPSession } from '@playwright/test';
import { execFileSync } from 'node:child_process';
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join } from 'node:path';
import { apiCtx } from './seed';
import { dist, keyFrameCosts, watchKeyFrameCosts } from './perf';

// ─────────────────────────────────────────────────────────────────────────────
// Threads (structured runtime) performance — numbers, not pass/fail. Real
// threads on the isolated daemon (otto-fake-codex: echo turns to build a 60- and
// a 300-turn history, `stream:<n>:<ms>:<tools>` for a long answer after tool
// calls); the 500-thread sidebar is a mocked list (UI-only). Opt in with
// OTTO_THREADS_RT_PERF=1; one JSON object per run goes to
// OTTO_THREADS_RT_PERF_OUT (default <tmp>/threads-rt-perf.json):
//   a. opening a 60 / 300-turn thread: first content, interactive, long tasks;
//   b. a 500-thread sidebar over 20 projects: first render, Search keystrokes,
//      switching thread;
//   c. a ~4,000-delta answer after 30 tool calls into the 300-turn thread:
//      frames over 50 ms, script time, and recomputation counts of the heavy
//      derivations (lib/perfCount.ts; zero on a build without the counters);
//   d. composer keystroke→paint: plain text, the slash popup, the mention menu;
//   e. six threads streaming for OTTO_THREADS_RT_PERF_SOAK_S (default 600 s; 0
//      skips): JS heap, DOM nodes, ottod RSS / CPU, requests and WS messages
//      per minute;
//   f. page load: requests and bytes;
//   g. phone (iPhone viewport, 4× CPU throttle): (a) and (d) again.
// Chromium only (long tasks, CDP metrics and CPU throttling).
// ─────────────────────────────────────────────────────────────────────────────

const ON = process.env.OTTO_THREADS_RT_PERF === '1';
const OUT = process.env.OTTO_THREADS_RT_PERF_OUT ?? join(tmpdir(), 'threads-rt-perf.json');
const SOAK_S = Number(process.env.OTTO_THREADS_RT_PERF_SOAK_S ?? '600');
test.describe.configure({ mode: 'serial' });
// No action may wait forever: a build without a control records null instead.
test.use({ actionTimeout: 15_000 });
test.setTimeout(SOAK_S * 1000 + 900_000);

const result: Record<string, unknown> = { at: new Date().toISOString() };
const record = (k: string, v: unknown): void => {
  result[k] = v;
  console.log(JSON.stringify({ [k]: v }));
};

let ctx: APIRequestContext;
let base = '';
let wsId = '';
let dir = '';
const ids: Record<string, string> = {};

async function thread(title: string, provider: 'claude' | 'codex' = 'codex'): Promise<string> {
  const r = await ctx.post(base + '/api/v1/workspaces/' + wsId + '/runtime-sessions', { data: { provider, cwd: dir, title } });
  expect(r.ok(), await r.text()).toBeTruthy();
  return (await r.json()).id as string;
}

async function idle(id: string, timeout = 120_000): Promise<void> {
  const t0 = Date.now();
  for (;;) {
    const s = (await (await ctx.get(base + '/api/v1/runtime-sessions/' + id)).json()) as { status: string };
    if (s.status === 'idle') return;
    if (Date.now() - t0 > timeout) throw new Error('thread never went idle');
    await new Promise((r) => setTimeout(r, 25));
  }
}

async function say(id: string, text: string): Promise<void> {
  expect((await ctx.post(base + '/api/v1/runtime-sessions/' + id + '/prompt', { data: { text } })).ok()).toBeTruthy();
}

/** A thread with `turns` finished echo turns (each turn: prompt + answer). */
async function history(title: string, turns: number, provider: 'claude' | 'codex' = 'codex'): Promise<string> {
  const id = await thread(title, provider);
  for (let i = 0; i < turns; i++) {
    await say(id, 'Turn ' + i + ': tighten the retry clamp in module ' + (i % 40) + ' and keep the API stable.');
    await idle(id);
  }
  return id;
}

test.beforeAll(async () => {
  if (!ON) return;
  ({ ctx, base } = await apiCtx());
  dir = mkdtempSync(join(tmpdir(), 'otto-rt-perf-'));
  writeFileSync(join(dir, 'README.md'), '# perf\n');
  const r = await ctx.post(base + '/api/v1/workspaces', { data: { name: 'RT perf', root_path: dir } });
  wsId = (await r.json()).id as string;
  ids.small = await thread('Perf warm-up');
  await say(ids.small, 'hello');
  await idle(ids.small);
  ids.t60 = await history('Perf sixty turns', 60);
  ids.t300 = await history('Perf three hundred turns', 300);
  ids.c60 = await history('Perf subagents', 60, 'claude');
});

test.afterAll(async () => {
  if (!ON) return;
  mkdirSync(dirname(OUT), { recursive: true });
  writeFileSync(OUT, JSON.stringify(result, null, 2));
  for (const id of Object.values(ids)) await ctx.post(base + '/api/v1/runtime-sessions/' + id + '/close').catch(() => {});
  await ctx?.dispose();
  if (dir) rmSync(dir, { recursive: true, force: true });
});

test.beforeEach(({}, info) => {
  test.skip(!ON, 'set OTTO_THREADS_RT_PERF=1');
  test.skip(info.project.name !== 'desktop-browser', 'Chromium (desktop-browser) only');
});

async function boot(page: Page, hash: string): Promise<void> {
  await page.addInitScript((id) => {
    localStorage.setItem('otto_workspace', id as string);
    localStorage.setItem('otto_firstrun_dismissed', '1');
  }, wsId);
  await page.goto('/#/threads/' + ids.small);
  await expect(page.getByTestId('runtime-thread-' + ids.small)).toBeVisible({ timeout: 60_000 });
  // The dev server compiles a module on its first import: warm the conversation.
  await page.evaluate(() => import(/* @vite-ignore */ '/src/modules/threads/runtime/RuntimeThread.svelte'));
  if (hash) await page.evaluate((h) => (location.hash = h), hash);
}

/** rAF-to-rAF deltas from now until `stop` (ms). */
async function startFrames(page: Page): Promise<void> {
  await page.evaluate(() => {
    const w = window as unknown as { __rf: number[]; __rfOn: boolean };
    w.__rf = [];
    w.__rfOn = true;
    let last = performance.now();
    const loop = (t: number): void => {
      if (!w.__rfOn) return;
      w.__rf.push(t - last);
      last = t;
      requestAnimationFrame(loop);
    };
    requestAnimationFrame((t) => {
      last = t;
      requestAnimationFrame(loop);
    });
  });
}
async function stopFrames(page: Page): Promise<number[]> {
  return page.evaluate(() => {
    const w = window as unknown as { __rf: number[]; __rfOn: boolean };
    w.__rfOn = false;
    return w.__rf;
  });
}
async function watchLt(page: Page): Promise<void> {
  await page.evaluate(() => {
    const w = window as unknown as { __lt: number[]; __ltObs?: PerformanceObserver };
    w.__lt = [];
    w.__ltObs?.disconnect();
    w.__ltObs = new PerformanceObserver((l) => {
      for (const e of l.getEntries()) w.__lt.push(e.duration);
    });
    w.__ltObs.observe({ type: 'longtask', buffered: false });
  });
}
const lts = (page: Page): Promise<number[]> => page.evaluate(() => (window as unknown as { __lt?: number[] }).__lt ?? []);
async function scriptMs(cdp: CDPSession): Promise<number> {
  const m = (await cdp.send('Performance.getMetrics')).metrics;
  return Math.round((m.find((x) => x.name === 'ScriptDuration')?.value ?? 0) * 1000);
}
const sumR = (xs: number[]): number => Math.round(xs.reduce((a, b) => a + b, 0));

/** (a): open `id` from the warm app; first content, interactive, long tasks. */
async function openThread(page: Page, id: string, last: string): Promise<Record<string, number>> {
  await page.evaluate((h) => (location.hash = h), '#/threads/' + ids.small);
  await expect(page.getByTestId('runtime-thread-' + ids.small)).toBeVisible();
  await watchLt(page);
  const t0 = Date.now();
  await page.evaluate((h) => (location.hash = h), '#/threads/' + id);
  const conv = page.getByTestId('runtime-thread-' + id);
  await expect(conv.getByText(last).last()).toBeVisible({ timeout: 30_000 });
  const firstContent = Date.now() - t0;
  // Interactive: the composer takes the caret and two quiet frames follow.
  await conv.getByTestId('runtime-composer').focus();
  await page.evaluate(async () => {
    let quiet = 0;
    let last = performance.now();
    while (quiet < 2) {
      const t = await new Promise<number>((r) => requestAnimationFrame(r));
      quiet = t - last < 50 ? quiet + 1 : 0;
      last = t;
    }
  });
  const interactive = Date.now() - t0;
  const l = await lts(page);
  return { firstContentMs: firstContent, interactiveMs: interactive, longTasks: l.length, longTaskMs: sumR(l), domNodes: await page.evaluate(() => document.getElementsByTagName('*').length) };
}

/** (d): keystroke→paint while typing plain text, a slash command, a mention. */
/** The dev server's error overlay (a stray runtime error elsewhere) would sit over the composer. */
const dropOverlay = (page: Page): Promise<void> => page.evaluate(() => document.querySelectorAll('vite-error-overlay').forEach((e) => e.remove()));

async function typing(page: Page, id: string): Promise<Record<string, unknown>> {
  await dropOverlay(page);
  const composer = page.getByTestId('runtime-thread-' + id).getByTestId('runtime-composer');
  await composer.click({ timeout: 10_000 });
  await watchKeyFrameCosts(page);
  // A mode a build cannot type in (a popup covering the box) is null, not a hang.
  const run = async (text: string): Promise<ReturnType<typeof dist> | null> => {
    try {
      await dropOverlay(page);
      await page.evaluate(() => ((window as unknown as { __kf: number[] }).__kf = []));
      await composer.pressSequentially(text, { delay: 40, timeout: 20_000 });
      const d = dist(await keyFrameCosts(page));
      await page.keyboard.press('Escape');
      await composer.fill('', { timeout: 10_000 });
      return d;
    } catch {
      await page.keyboard.press('Escape').catch(() => {});
      return null;
    }
  };
  return { plain: await run('the quick brown fox jumps over the lazy dog'), slash: await run('/rev'), mention: await run('@READ') };
}

test('a. opening a 60- and a 300-turn thread; d. composer typing', async ({ page }) => {
  await boot(page, '');
  record('open60', await openThread(page, ids.t60, 'Turn 59: tighten'));
  record('open300', await openThread(page, ids.t300, 'Turn 299: tighten'));
  record('typing300', await typing(page, ids.t300));
});

test('c. streaming a long answer after 30 tool calls into the 300-turn thread', async ({ page }) => {
  // Wide enough for the details card to float beside the conversation.
  await page.setViewportSize({ width: 1600, height: 1000 });
  await boot(page, '#/threads/' + ids.t300);
  const conv = page.getByTestId('runtime-thread-' + ids.t300);
  await expect(conv.getByText('Turn 299: tighten').last()).toBeVisible({ timeout: 30_000 });
  const cdp = await page.context().newCDPSession(page);
  await cdp.send('Performance.enable');
  await page.evaluate(() => ((window as unknown as { __ottoPerfCounts: Record<string, number> }).__ottoPerfCounts = {}));
  const s0 = await scriptMs(cdp);
  const l0 = (await cdp.send('Performance.getMetrics')).metrics.find((m) => m.name === 'LayoutCount')!.value;
  const profile = !!process.env.OTTO_THREADS_RT_PERF_PROFILE;
  if (profile) await cdp.send('Profiler.enable').then(() => cdp.send('Profiler.start'));
  await startFrames(page);
  const t0 = Date.now();
  await say(ids.t300, 'stream:4000:3:30');
  await expect(conv.getByText('c3999', { exact: false }).last()).toBeVisible({ timeout: 120_000 });
  await idle(ids.t300);
  const wall = Date.now() - t0;
  if (profile) writeFileSync(process.env.OTTO_THREADS_RT_PERF_PROFILE!, JSON.stringify((await cdp.send('Profiler.stop')).profile));
  const frames = await stopFrames(page);
  const script = (await scriptMs(cdp)) - s0;
  const layouts = (await cdp.send('Performance.getMetrics')).metrics.find((m) => m.name === 'LayoutCount')!.value - l0;
  const counts = await page.evaluate(() => (window as unknown as { __ottoPerfCounts: Record<string, number> }).__ottoPerfCounts);
  record('stream300', { layouts, wallMs: wall, frames: frames.length, framesOver50: frames.filter((d) => d > 50).length, worstFrameMs: Math.round(Math.max(0, ...frames)), scriptMs: script, counts });
});

test('c2. a long answer streaming while three subagents work (Claude)', async ({ page }) => {
  await page.setViewportSize({ width: 1600, height: 1000 });
  await boot(page, '#/threads/' + ids.c60);
  const conv = page.getByTestId('runtime-thread-' + ids.c60);
  await expect(conv.getByText('Turn 59: tighten').last()).toBeVisible({ timeout: 30_000 });
  for (let i = 0; i < 3; i++) {
    await say(ids.c60, 'agent 90');
    await idle(ids.c60);
  }
  const cdp = await page.context().newCDPSession(page);
  await cdp.send('Performance.enable');
  await page.evaluate(() => ((window as unknown as { __ottoPerfCounts: Record<string, number> }).__ottoPerfCounts = {}));
  const s0 = await scriptMs(cdp);
  await startFrames(page);
  await say(ids.c60, 'stream 4000 3');
  await expect(conv.getByText('c3999', { exact: false }).last()).toBeVisible({ timeout: 120_000 });
  const frames = await stopFrames(page);
  const script = (await scriptMs(cdp)) - s0;
  const counts = await page.evaluate(() => (window as unknown as { __ottoPerfCounts: Record<string, number> }).__ottoPerfCounts);
  record('streamSubagents', { frames: frames.length, framesOver50: frames.filter((d) => d > 50).length, worstFrameMs: Math.round(Math.max(0, ...frames)), scriptMs: script, counts });
});

test('b. a 500-thread sidebar over 20 projects: first render, Search, switching', async ({ page }) => {
  const rows = Array.from({ length: 500 }, (_, i) => ({
    id: 'mock-' + i, workspace_id: wsId, provider: i % 3 ? 'codex' : 'claude', title: 'Mock thread ' + i + ' ' + ['retry', 'cache', 'auth', 'sync'][i % 4],
    cwd: '/tmp/perf-project-' + (i % 20), status: i % 25 === 0 ? 'awaiting_permission' : i % 10 === 0 ? 'working' : 'idle', access_mode: 'full-access',
    created_by: 'root', created_at: new Date(Date.now() - i * 60_000).toISOString(), last_active_at: new Date(Date.now() - i * 60_000).toISOString(),
    last_sequence: 1, parent_id: null, meta: {}, model: null, capabilities: {},
  }));
  const snapshot = { kind: 'snapshot', snapshot_sequence: 1, snapshot: { provider_session: { status: 'ready', cwd: '/tmp', model: null, updated_at: new Date().toISOString(), last_error: null }, provider_thread: null, turns: [], turn_items: [], runtime_requests: [], plans: [] } };
  await page.route('**/api/v1/workspaces/*/runtime-sessions', (r) => (r.request().method() === 'GET' ? r.fulfill({ json: rows }) : r.fallback()));
  await page.route('**/api/v1/runtime-sessions?*', (r) => r.fulfill({ json: rows }));
  await page.route('**/api/v1/runtime-sessions/mock-*/events**', (r) => r.fulfill({ json: snapshot }));
  await page.route(/\/api\/v1\/runtime-sessions\/mock-\d+$/, (r) => r.fulfill({ json: rows.find((x) => r.request().url().endsWith('/' + x.id)) }));
  await page.addInitScript((id) => {
    localStorage.setItem('otto_workspace', id as string);
    localStorage.setItem('otto_firstrun_dismissed', '1');
  }, wsId);
  await page.goto('/#/threads/' + ids.small);
  await page.evaluate(() => import(/* @vite-ignore */ '/src/modules/threads/ThreadSidebar.svelte'));
  await page.goto('about:blank');
  let rowReads = 0;
  page.on('request', (rq) => {
    if (/\/runtime-sessions\/[^/]+\/events\?/.test(rq.url())) rowReads++;
  });
  const t0 = Date.now();
  await page.goto('/#/threads/mock-3');
  const side = page.getByTestId('threads-sidebar');
  await expect(side.getByText('Mock thread 3 ', { exact: false }).first()).toBeVisible({ timeout: 60_000 });
  const firstRender = Date.now() - t0;
  const sideNodes = await side.evaluate((el) => el.getElementsByTagName('*').length);
  await page.waitForTimeout(2000); // ui-guards: allow — let per-row reads settle before counting
  const eventReads = rowReads;
  // Search: per-keystroke cost.
  const search = side.getByRole('searchbox').or(side.getByPlaceholder('Search')).first();
  let searchKeys: ReturnType<typeof dist> | null = null;
  try {
    await search.click();
    await watchKeyFrameCosts(page);
    await search.pressSequentially('cache 1', { delay: 60 });
    searchKeys = dist(await keyFrameCosts(page));
    await search.fill('');
  } catch {
    searchKeys = null;
  }
  // Switching: click 10 rows in turn, time to the pane.
  const sw: number[] = [];
  for (let i = 1; i <= 10; i++) {
    const id = 'mock-' + i * 7;
    const row = side.getByTestId('thread-row-' + id);
    if (!(await row.count())) continue;
    const s0 = Date.now();
    try {
      await row.locator('.tr-main').first().click();
      await expect(page.getByTestId('runtime-thread-' + id)).toBeVisible();
      sw.push(Date.now() - s0);
    } catch {
      /* not reachable on this build */
    }
  }
  record('sidebar500', { firstRenderMs: firstRender, sidebarNodes: sideNodes, eventReads, searchKey: searchKeys, switchMs: dist(sw) });
});

test('f. page load: requests and bytes', async ({ page }) => {
  await page.addInitScript((id) => {
    localStorage.setItem('otto_workspace', id as string);
    localStorage.setItem('otto_firstrun_dismissed', '1');
  }, wsId);
  await page.goto('/#/threads/' + ids.small);
  await expect(page.getByTestId('runtime-thread-' + ids.small)).toBeVisible({ timeout: 60_000 });
  await page.goto('about:blank');
  const reqs: { url: string; bytes: number }[] = [];
  page.on('requestfinished', async (rq) => {
    const s = await rq.sizes().catch(() => null);
    reqs.push({ url: rq.url(), bytes: s ? s.responseBodySize : 0 });
  });
  await page.goto('/#/threads/' + ids.t60);
  await expect(page.getByTestId('runtime-thread-' + ids.t60).getByText('Turn 59: tighten').last()).toBeVisible({ timeout: 60_000 });
  await page.waitForLoadState('networkidle').catch(() => {});
  const api = reqs.filter((r) => r.url.includes('/api/v1/'));
  record('pageLoad', { requests: reqs.length, apiRequests: api.length, apiBytes: sumR(api.map((r) => r.bytes)), totalBytes: sumR(reqs.map((r) => r.bytes)) });
});

test('g. phone, 4× CPU throttle: opening the threads and typing', async ({ browser }) => {
  const { defaultBrowserType: _d, ...iphone } = devices['iPhone 14 Pro Max'];
  const context = await browser.newContext({ ...iphone, storageState: test.info().project.use.storageState as string });
  const page = await context.newPage();
  await boot(page, '');
  const cdp = await context.newCDPSession(page);
  await cdp.send('Emulation.setCPUThrottlingRate', { rate: 4 });
  record('phoneOpen60', await openThread(page, ids.t60, 'Turn 59: tighten'));
  record('phoneOpen300', await openThread(page, ids.t300, 'Turn 299: tighten'));
  record('phoneTyping300', await typing(page, ids.t300));
  await context.close();
});

test('e. six threads streaming at once: memory, CPU, requests', async ({ page }) => {
  test.skip(SOAK_S <= 0, 'soak off');
  const six: string[] = [];
  for (let i = 0; i < 6; i++) six.push(await thread('Perf soak ' + i));
  ids.soak = six.join(',');
  const apiPerMin: number[] = [];
  const wsPerMin: number[] = [];
  let api = 0;
  let ws = 0;
  const paths: Record<string, number> = {};
  page.on('request', (r) => {
    if (!r.url().includes('/api/v1/')) return;
    api++;
    const p = r.method() + ' ' + new URL(r.url()).pathname.replace(/\/api\/v1/, '').replace(/[0-9A-HJKMNP-TV-Z]{26}|[0-9a-f-]{36}/g, ':id');
    paths[p] = (paths[p] ?? 0) + 1;
  });
  page.on('websocket', (s) => s.on('framereceived', () => ws++));
  // API latency as the page sees it (request start → response end).
  const lat: number[] = [];
  page.on('requestfinished', (r) => {
    if (!r.url().includes('/api/v1/')) return;
    const t = r.timing();
    if (t.responseEnd > 0) lat.push(Math.round(t.responseEnd));
  });
  await boot(page, '#/threads/' + six[0]);
  const cdp = await page.context().newCDPSession(page);
  const pid = Number(execFileSync('lsof', ['-ti', 'tcp:' + (process.env.OTTO_E2E_PORT ?? '7799'), '-sTCP:LISTEN']).toString().trim().split('\n')[0]);
  const sample = async (label: string): Promise<void> => {
    await cdp.send('HeapProfiler.collectGarbage');
    const heap = (await cdp.send('Runtime.getHeapUsage')).usedSize;
    const ps = execFileSync('ps', ['-o', 'rss=,%cpu=', '-p', String(pid)]).toString().trim().split(/\s+/);
    record('soak_' + label, { heapMB: Math.round(heap / 1e5) / 10, domNodes: await page.evaluate(() => document.getElementsByTagName('*').length), ottodRssMB: Math.round(Number(ps[0]) / 1024), ottodCpu: Number(ps[1]) });
  };
  const loop = async (id: string, until: number): Promise<void> => {
    while (Date.now() < until) {
      await say(id, 'stream:1500:20:5');
      await idle(id, 180_000);
    }
  };
  const t0 = Date.now();
  const until = t0 + SOAK_S * 1000;
  await sample('start');
  const loops = Promise.all(six.map((id) => loop(id, until)));
  const half = Math.round(SOAK_S / 2);
  for (let m = 1; m * 60 <= SOAK_S; m++) {
    await page.waitForTimeout(Math.max(0, t0 + m * 60_000 - Date.now())); // ui-guards: allow — a per-minute sampling clock
    apiPerMin.push(api);
    wsPerMin.push(ws);
    api = 0;
    ws = 0;
    if (m * 60 === half || (half < 60 && m === 1)) await sample('mid');
  }
  await loops;
  await sample('end');
  record('soakPerMin', { apiRequests: dist(apiPerMin), wsMessages: dist(wsPerMin) });
  record('soakApiLatencyMs', dist(lat));
  record('soakApiPaths', Object.fromEntries(Object.entries(paths).sort((a, b) => b[1] - a[1])));
});
