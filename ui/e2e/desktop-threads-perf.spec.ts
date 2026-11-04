import { test, expect, type APIRequestContext, type Page } from '@playwright/test';
import { appendFileSync, mkdtempSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { apiCtx } from './seed';
import { assistantText, toolResult, toolUse, userPrompt } from './chat-fixture';
import { dist, domCount, frameDeltas, isDesktopProject, keyFrameCosts, requestLog, watchKeyFrameCosts, watchFatalUiErrors } from './perf';

// ─────────────────────────────────────────────────────────────────────────────
// Threads runtime performance (measurement harness, not a gate):
//   1. #/threads time-to-interactive with 1 / 20 / 100 threads in the sidebar;
//   2. opening a 500 / 2,000-exchange transcript (prompts, prose, Edit tool
//      calls with structured diffs): first turn painted, DOM nodes, scroll
//      frame deltas;
//   3. switching between two long threads: time and transcript GETs per switch;
//   4. composer keystroke→paint while the same thread streams;
//   5. memory with 3 streaming threads in split view, and after closing them;
//   6. idle main-thread work with Threads open;
//   7. idle requests per minute on Threads.
// Opt in with OTTO_THREADS_PERF=1; results append as JSON lines to
// OTTO_THREADS_PERF_OUT (default: <tmp>/threads-perf.jsonl). Durations:
// OTTO_THREADS_PERF_MEM_S (default 600), OTTO_THREADS_PERF_IDLE_S (default 60).
// ─────────────────────────────────────────────────────────────────────────────

const ON = process.env.OTTO_THREADS_PERF === '1';
const OUT = process.env.OTTO_THREADS_PERF_OUT ?? join(tmpdir(), 'threads-perf.jsonl');
const MEM_S = Number(process.env.OTTO_THREADS_PERF_MEM_S ?? '600');
const IDLE_S = Number(process.env.OTTO_THREADS_PERF_IDLE_S ?? '60');
test.setTimeout(Math.max(MEM_S, IDLE_S) * 1000 + 600_000);
test.describe.configure({ mode: 'serial' });

let ctx: APIRequestContext;
let base = '';
let dir = '';
const created: string[] = [];

function record(project: string, metric: string, value: unknown): void {
  const line = JSON.stringify({ project, metric, value });
  console.log(line);
  appendFileSync(OUT, line + '\n');
}

/** `n` exchanges: prompt, prose with a code block, an Edit with a diff, its
 *  result, a closing line — the shape of a real working session. */
function longTranscript(n: number, tag: string): string {
  let s = '';
  for (let i = 0; i < n; i++) {
    const req = `${tag}${i}`;
    s += userPrompt(`Step ${i}: tighten the retry clamp in module ${i % 40} and keep the API stable.`);
    s += assistantText(req, `Looking at \`retry_${i % 40}.rs\`:\n\n\`\`\`rust\nlet capped = exp.min(max);\n(capped + jitter(capped / 10)).min(max)\n\`\`\`\n\nApplying the clamp now.`);
    const id = `toolu_${tag}_${i}`;
    const file = `/repo/otto/crates/otto-net/src/retry_${i % 40}.rs`;
    s += toolUse(req, id, 'Edit', { file_path: file, old_string: '    capped + jitter(capped / 10)', new_string: '    (capped + jitter(capped / 10)).min(max)' });
    s += toolResult(id, `The file ${file} has been updated successfully.`, {
      filePath: file,
      oldString: '    capped + jitter(capped / 10)',
      newString: '    (capped + jitter(capped / 10)).min(max)',
      structuredPatch: [{ oldStart: 80, oldLines: 3, newStart: 80, newLines: 3, lines: ['     let capped = exp.min(max);', '-    capped + jitter(capped / 10)', '+    (capped + jitter(capped / 10)).min(max)', ' }'] }],
    });
    s += assistantText(req, `Done with step ${i}; tests for module ${i % 40} pass.`);
  }
  return s;
}

async function seedWs(name: string): Promise<string> {
  const root = mkdtempSync(join(dir, `${name}-`));
  const r = await ctx.post(`${base}/api/v1/workspaces`, { data: { name, root_path: root } });
  expect(r.ok(), await r.text()).toBeTruthy();
  return (await r.json()).id as string;
}

async function seedThread(wsId: string, title: string, transcript: string): Promise<{ id: string; path: string }> {
  const path = join(dir, `${title.replace(/\W+/g, '-')}-${created.length}.jsonl`);
  writeFileSync(path, transcript);
  const r = await ctx.post(`${base}/api/v1/workspaces/${wsId}/sessions`, {
    data: { kind: 'agent', provider: 'shell', title, cwd: dir, meta: { origin: 'e2e', surface: 'threads', nested_provider: 'claude', e2e_transcript_path: path } },
  });
  expect(r.ok(), await r.text()).toBeTruthy();
  const id = (await r.json()).id as string;
  created.push(id);
  return { id, path };
}

async function openThreads(page: Page, wsId: string, hash = '#/threads'): Promise<void> {
  await page.addInitScript((id) => {
    localStorage.setItem('otto_workspace', id as string);
    localStorage.setItem('otto_firstrun_dismissed', '1');
  }, wsId);
  // A hash-only goto would not reload the app: start every open from a blank page.
  await page.goto('about:blank');
  await page.goto(`/${hash}`);
}

/** Wall time from the click to the newest turn painted in `pane`. */
async function openAndTime(page: Page, rowTestId: string, paneId: string): Promise<number> {
  const t0 = await page.evaluate(() => performance.now());
  await page.getByTestId(rowTestId).locator('.tr-main').click();
  await page.locator(`[data-testid="thread-pane-${paneId}"] .turn`).last().waitFor({ state: 'visible', timeout: 60_000 });
  return page.evaluate(async (start) => {
    await new Promise((r) => requestAnimationFrame(() => requestAnimationFrame(r)));
    return performance.now() - start;
  }, t0);
}

/** A fixed measurement window (idle time, a sample interval), not a wait for a condition. */
const hold = (page: Page, ms: number): Promise<void> => page.waitForTimeout(ms); // ui-guards: allow — measurement window

const seeded: Record<string, string> = {};
const longIds: Record<string, { id: string; path: string }> = {};

test.beforeAll(async () => {
  if (!ON) return;
  ({ ctx, base } = await apiCtx());
  dir = mkdtempSync(join(tmpdir(), 'otto-threads-perf-'));
  for (const n of [1, 20, 100]) {
    const wsId = await seedWs(`Threads ${n}`);
    seeded[n] = wsId;
    for (let i = 0; i < n; i++) await seedThread(wsId, `t${n}-${i}`, longTranscript(2, `s${n}_${i}_`));
  }
  const big = await seedWs('Threads long');
  seeded.long = big;
  longIds[500] = await seedThread(big, 'long-500', longTranscript(500, 'a'));
  longIds[2000] = await seedThread(big, 'long-2000', longTranscript(2000, 'b'));
  for (let i = 0; i < 3; i++) longIds[`live${i}`] = await seedThread(big, `live-${i}`, longTranscript(40, `l${i}_`));
});

test.afterAll(async () => {
  if (!ON) return;
  for (const id of created) await ctx.delete(`${base}/api/v1/sessions/${id}`).catch(() => {});
  rmSync(dir, { recursive: true, force: true });
});

test.beforeEach(async ({ page }, info) => {
  test.skip(!ON, 'measurement harness: set OTTO_THREADS_PERF=1');
  test.skip(!isDesktopProject(info.project.name), 'desktop projects only');
  watchFatalUiErrors(page);
});

test('1. time to interactive with 1 / 20 / 100 threads', async ({ page }, info) => {
  for (const n of [1, 20, 100]) {
    await openThreads(page, seeded[n]);
    const rows = page.locator('[data-testid^="thread-row-"]');
    await expect(rows).toHaveCount(n, { timeout: 60_000 });
    const tti = await page.evaluate(async () => {
      await new Promise((r) => requestAnimationFrame(() => requestAnimationFrame(r)));
      return performance.now();
    });
    record(info.project.name, `tti_ms_n${n}`, Math.round(tti));
    record(info.project.name, `dom_nodes_sidebar_n${n}`, await domCount(page));
  }
});

test('2+3. open long transcripts, scroll, and switch between them', async ({ page }, info) => {
  await openThreads(page, seeded.long);
  const gets = requestLog(page, /\/transcript/);
  for (const n of [500, 2000]) {
    const { id } = longIds[n];
    gets.reset();
    const ms = await openAndTime(page, `thread-row-${id}`, id);
    record(info.project.name, `open_ms_${n}`, Math.round(ms));
    await hold(page, 1000);
    record(info.project.name, `open_transcript_gets_${n}`, gets.stats().count);
    record(info.project.name, `open_transcript_kb_${n}`, Math.round(gets.stats().bytesReceived / 1024));
    record(info.project.name, `dom_nodes_open_${n}`, await domCount(page));
    record(info.project.name, `turns_mounted_${n}`, await domCount(page, `[data-testid="thread-pane-${id}"] .turn`));
    const list = `[data-testid="thread-pane-${id}"] .conv-list`;
    const fd = await frameDeltas(page, async () => {
      for (let i = 0; i < 30; i++) {
        await page.locator(list).hover();
        await page.mouse.wheel(0, -600);
        await hold(page, 16);
      }
    });
    record(info.project.name, `scroll_frames_${n}`, fd);
  }
  // Switch A ↔ B: each switch's time and its transcript GETs.
  const a = longIds[500].id;
  const b = longIds[2000].id;
  const times: number[] = [];
  const perSwitch: number[] = [];
  const work: number[] = [];
  const cdp = info.project.name === 'desktop-browser' ? await page.context().newCDPSession(page) : null;
  await cdp?.send('Performance.enable');
  const taskS = async (): Promise<number> => {
    if (!cdp) return 0;
    const { metrics } = await cdp.send('Performance.getMetrics');
    return metrics.find((m: { name: string }) => m.name === 'TaskDuration')?.value ?? 0;
  };
  for (let i = 0; i < 6; i++) {
    const to = i % 2 === 0 ? a : b;
    gets.reset();
    const w0 = await taskS();
    times.push(await openAndTime(page, `thread-row-${to}`, to));
    await hold(page, 1500);
    if (cdp) work.push(Math.round(((await taskS()) - w0) * 1000));
    perSwitch.push(gets.stats().count);
    if (i === 5) record(info.project.name, 'switch_transcript_paths', gets.stats().paths.map((x) => x.replace(/[0-9A-Z]{26}/g, ':id')));
  }
  if (cdp && process.env.OTTO_THREADS_PERF_PROFILE === '1') {
    // Self time by function over one more switch (where the mount cost goes).
    await cdp.send('Profiler.enable');
    await cdp.send('Profiler.setSamplingInterval', { interval: 100 });
    await cdp.send('Profiler.start');
    await openAndTime(page, `thread-row-${a}`, a);
    await hold(page, 1500);
    const { profile } = await cdp.send('Profiler.stop');
    const self = new Map<string, number>();
    const dt = profile.timeDeltas ?? [];
    const byId = new Map(profile.nodes.map((n: { id: number }) => [n.id, n]));
    (profile.samples ?? []).forEach((id: number, i: number) => {
      const n = byId.get(id) as unknown as { callFrame: { functionName: string; url: string; lineNumber: number } };
      const k = `${n.callFrame.functionName || '(anon)'} ${n.callFrame.url.split('/').pop()}:${n.callFrame.lineNumber}`;
      self.set(k, (self.get(k) ?? 0) + (dt[i] ?? 0) / 1000);
    });
    record(info.project.name, 'switch_profile_top', [...self.entries()].sort((x, y) => y[1] - x[1]).slice(0, 30).map(([k, v]) => `${v.toFixed(1)}ms ${k}`));
  }
  record(info.project.name, 'switch_ms', dist(times));
  if (cdp) record(info.project.name, 'switch_main_thread_ms', dist(work));
  const rt = await page.evaluate(() =>
    performance.getEntriesByType('resource').filter((e) => /\/transcript\?/.test(e.name)).map((e) => {
      const r = e as PerformanceResourceTiming;
      return { server: Math.round(r.responseStart - r.requestStart), total: Math.round(r.duration), kb: Math.round(r.encodedBodySize / 1024) };
    }));
  record(info.project.name, 'transcript_get_timing', { server: dist(rt.map((x) => x.server)), total: dist(rt.map((x) => x.total)), kb: dist(rt.map((x) => x.kb)) });
  record(info.project.name, 'switch_transcript_gets', perSwitch);
  gets.stop();
});

test('4. composer typing while the thread streams', async ({ page }, info) => {
  await openThreads(page, seeded.long);
  const { id, path } = longIds.live0;
  await openAndTime(page, `thread-row-${id}`, id);
  let k = 0;
  const stream = setInterval(() => appendFileSync(path, assistantText(`stream${k}`, `streamed line ${k++} ${'lorem ipsum '.repeat(8)}`)), 200);
  try {
    const box = page.locator(`[data-testid="thread-pane-${id}"] textarea`).first();
    await box.click();
    await watchKeyFrameCosts(page);
    await page.keyboard.type('the quick brown fox jumps over the lazy dog '.repeat(3), { delay: 60 });
    const costs = await keyFrameCosts(page);
    record(info.project.name, 'type_while_streaming_ms', dist(costs));
  } finally {
    clearInterval(stream);
  }
});

test('5. memory with 3 streaming threads in split view, then closed', async ({ page }, info) => {
  await openThreads(page, seeded.long);
  const lives = [longIds.live0, longIds.live1, longIds.live2];
  await openAndTime(page, `thread-row-${lives[0].id}`, lives[0].id);
  for (const l of lives.slice(1)) {
    await page.getByTestId(`thread-row-${l.id}`).locator('.tr-main').click({ modifiers: ['Meta'] });
    await page.locator(`[data-testid="thread-pane-${l.id}"] .turn`).last().waitFor({ timeout: 60_000 });
  }
  const cdp = info.project.name === 'desktop-browser' ? await page.context().newCDPSession(page) : null;
  const heap = async (): Promise<Record<string, number> | null> => {
    if (!cdp) return null;
    await cdp.send('HeapProfiler.collectGarbage');
    await cdp.send('Performance.enable');
    const { metrics } = await cdp.send('Performance.getMetrics');
    const m = Object.fromEntries(metrics.map((x: { name: string; value: number }) => [x.name, x.value]));
    return { heapMB: +(m.JSHeapUsedSize / 2 ** 20).toFixed(1), nodes: m.Nodes, listeners: m.JSEventListeners, docs: m.Documents };
  };
  let k = 0;
  const stream = setInterval(() => {
    for (const l of lives) appendFileSync(l.path, assistantText(`m${k}`, `memory line ${k} ${'lorem ipsum '.repeat(6)}`));
    k++;
  }, 500);
  const panes = async (): Promise<number[]> =>
    page.evaluate((ids) => ids.map((id) => document.querySelectorAll(`[data-testid="thread-pane-${id}"] .turn`).length), lives.map((l) => l.id));
  const series: unknown[] = [];
  try {
    const t0 = Date.now();
    series.push({ s: 0, ...(await heap()), dom: await domCount(page), turns: await panes() });
    while (Date.now() - t0 < MEM_S * 1000) {
      await hold(page, Math.min(60_000, MEM_S * 1000));
      series.push({ s: Math.round((Date.now() - t0) / 1000), ...(await heap()), dom: await domCount(page), turns: await panes() });
      if (series.length === 4) await page.screenshot({ path: OUT.replace(/\.jsonl$/, '-mem-3min.png') }).catch(() => {});
    }
  } finally {
    clearInterval(stream);
  }
  record(info.project.name, 'mem_series_3_streaming', series);
  for (const l of lives) {
    const close = page.getByTestId(`pane-close-${l.id}`);
    if (await close.count()) await close.click();
  }
  await page.goto('/#/home');
  await hold(page, 3000);
  record(info.project.name, 'mem_after_close', { ...(await heap()), dom: await domCount(page) });
});

test('6+7. idle work and requests on Threads', async ({ page }, info) => {
  await openThreads(page, seeded[20]);
  await expect(page.locator('[data-testid^="thread-row-"]')).toHaveCount(20, { timeout: 60_000 });
  const first = page.locator('[data-testid^="thread-row-"]').first();
  await first.locator('.tr-main').click();
  await hold(page, 5000);
  const reqs = requestLog(page, /\/api\/v1\//);
  const cdp = info.project.name === 'desktop-browser' ? await page.context().newCDPSession(page) : null;
  const metrics = async (): Promise<Record<string, number>> => {
    if (!cdp) return {};
    await cdp.send('Performance.enable');
    const { metrics: m } = await cdp.send('Performance.getMetrics');
    return Object.fromEntries(m.map((x: { name: string; value: number }) => [x.name, x.value]));
  };
  const before = await metrics();
  await hold(page, IDLE_S * 1000);
  const after = await metrics();
  const anims = await page.evaluate(() => {
    return document.getAnimations().filter((a) => a.playState === 'running').map((a) => (a as CSSAnimation).animationName ?? a.constructor.name);
  });
  const s = reqs.stats();
  reqs.stop();
  const perMin = (60 / IDLE_S) * s.count;
  const byPath: Record<string, number> = {};
  for (const p of s.paths) {
    const key = p.replace(/[0-9A-Z]{26}/g, ':id');
    byPath[key] = (byPath[key] ?? 0) + 1;
  }
  record(info.project.name, 'idle_requests_per_min', { perMin: +perMin.toFixed(1), byPath });
  record(info.project.name, 'idle_running_animations', anims);
  if (cdp) {
    const d = (k: string) => +((after[k] ?? 0) - (before[k] ?? 0)).toFixed(3);
    record(info.project.name, 'idle_main_thread_s', { script: d('ScriptDuration'), task: d('TaskDuration'), layout: d('LayoutDuration'), recalc: d('RecalcStyleDuration'), cpuPct: +((d('TaskDuration') / IDLE_S) * 100).toFixed(2) });
  }
});
