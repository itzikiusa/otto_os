import { test, expect, type APIRequestContext, type Page } from '@playwright/test';
import { appendFileSync, mkdtempSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { apiCtx, seedWorkspace } from './seed';
import { isDesktopProject } from './perf';

// ─────────────────────────────────────────────────────────────────────────────
// Conversation live-delta regression gates (GAPS_TO_9_5 §4, I3 A1).
//
// Real daemon, real live tail: a seeded inert shell session points at a
// synthetic Claude JSONL (the `OTTO_E2E=1` `meta.e2e_transcript_path` hook) of
// 300 turns × ~3 KB. With the chat open, the spec appends 20 assistant turns
// 700 ms apart (the tail's poll) and asserts, per delta:
//   • main-thread cost p95 < 5 ms (the rAF interval that absorbed the delta,
//     minus one frame);
//   • ≤ 1 markdown render per delta (mdCache render counter — every unchanged
//     block is a cache hit);
// then appends one 70 KB tool result — over the 64 KB event cap — and asserts
// the delta arrives trimmed (0 `GET …/transcript` re-fetches) and the step's
// full output loads lazily through `GET …/transcript/tool/{id}` on expand.
// Probes: `window.__ottoMdProbe` (Markdown.svelte registers its cache only
// when present).
// ─────────────────────────────────────────────────────────────────────────────

test.setTimeout(180_000);

const SID = '00000000-0000-4000-8000-00000000c0de';
const TS = '2026-09-27T12:00:00Z';
const filler = (i: number) =>
  `Paragraph ${i}. ` + 'The quick brown fox jumps over the lazy dog while the build keeps going. '.repeat(40);

function user(id: string, text: string): string {
  return JSON.stringify({ type: 'user', uuid: id, sessionId: SID, timestamp: TS, message: { role: 'user', content: text } }) + '\n';
}
function assistant(id: string, text: string): string {
  return (
    JSON.stringify({
      type: 'assistant',
      uuid: `u-${id}`,
      requestId: `req-${id}`,
      sessionId: SID,
      timestamp: TS,
      message: { id: `msg-${id}`, role: 'assistant', model: 'claude-e2e', content: [{ type: 'text', text }], stop_reason: 'end_turn', usage: { input_tokens: 1, output_tokens: 1 } },
    }) + '\n'
  );
}
function toolUse(id: string, toolId: string): string {
  return (
    JSON.stringify({
      type: 'assistant',
      uuid: `u-${id}`,
      requestId: `req-${id}`,
      sessionId: SID,
      timestamp: TS,
      message: { id: `msg-${id}`, role: 'assistant', model: 'claude-e2e', content: [{ type: 'tool_use', id: toolId, name: 'Bash', input: { command: 'cat big.log' } }], stop_reason: 'tool_use', usage: { input_tokens: 1, output_tokens: 1 } },
    }) + '\n'
  );
}
function toolResult(id: string, toolId: string, text: string): string {
  return JSON.stringify({ type: 'user', uuid: id, sessionId: SID, timestamp: TS, message: { role: 'user', content: [{ type: 'tool_result', tool_use_id: toolId, content: text }] } }) + '\n';
}

let root: APIRequestContext;
let base = '';
let wsId = '';
let session = '';
let dir = '';
let transcript = '';

test.beforeEach(async ({ page }, info) => {
  test.skip(!isDesktopProject(info.project.name), 'desktop projects only');
  // Under Playwright WebKit the seeded session never reaches the session list
  // ("No sessions"), so the gate can't start there yet — Chromium only until
  // the fixture is fixed (see perf-plan round3 fix-w2c).
  test.skip(info.project.name === 'desktop-webkit', 'fixture does not load under WebKit yet');
  ({ ctx: root, base } = await apiCtx());
  wsId = await seedWorkspace(root, base);
  dir = mkdtempSync(join(tmpdir(), 'otto-conv-perf-'));
  transcript = join(dir, `${SID}.jsonl`);
  let body = '';
  for (let i = 0; i < 150; i++) body += user(`q${i}`, `Question ${i}: ${filler(i).slice(0, 600)}`) + assistant(`a${i}`, filler(i));
  writeFileSync(transcript, body);
  const created = await root.post(`${base}/api/v1/workspaces/${wsId}/sessions`, {
    data: { kind: 'agent', provider: 'shell', title: 'ConvPerf', cwd: dir, meta: { origin: 'e2e', nested_provider: 'claude', e2e_transcript_path: transcript } },
  });
  expect(created.ok(), await created.text()).toBeTruthy();
  session = (await created.json()).id as string;
  await expect.poll(async () => (await (await root.get(`${base}/api/v1/sessions/${session}`)).json()).live).toBe(true);

  await page.addInitScript((id) => {
    localStorage.setItem('otto_workspace', id as string);
    localStorage.setItem('otto_firstrun_dismissed', '1');
    (window as unknown as { __ottoMdProbe: object }).__ottoMdProbe = {};
  }, wsId);
  await page.goto(`/#/agents/${session}`);
  await expect(page.locator('.pane-head', { hasText: 'ConvPerf' }).first()).toBeVisible({ timeout: 20_000 });
  await page.locator('.view-seg button', { hasText: 'Chat' }).click();
  await expect(page.locator('.conv[data-loaded="true"]')).toBeVisible({ timeout: 30_000 });
  await expect(page.locator('.conv').getByText('Paragraph 149.').first()).toBeVisible({ timeout: 30_000 });
});

test.afterEach(async () => {
  if (session && root) await root.delete(`${base}/api/v1/sessions/${session}`).catch(() => {});
  session = '';
  await root?.dispose();
  if (dir) rmSync(dir, { recursive: true, force: true });
  dir = '';
});

const renders = (page: Page) =>
  page.evaluate(() => (window as unknown as { __ottoMdProbe: { cache?: { renders: number } } }).__ottoMdProbe.cache?.renders ?? 0);

test('20 live deltas over 300 turns: p95 < 5 ms main thread, ≤ 1 markdown render each', async ({ page }) => {
  // The tail's initial fold must land before the first append.
  await page.waitForTimeout(2000);
  await page.evaluate(() => {
    const w = window as unknown as { __frames: number[]; __muts: number[] };
    w.__frames = [];
    w.__muts = [];
    const loop = (t: number): void => {
      w.__frames.push(t);
      requestAnimationFrame(loop);
    };
    requestAnimationFrame(loop);
    new MutationObserver(() => w.__muts.push(performance.now())).observe(document.querySelector('.conv')!, {
      childList: true,
      subtree: true,
      characterData: true,
    });
  });
  const before = await renders(page);
  const DELTAS = 20;
  for (let i = 0; i < DELTAS; i++) {
    appendFileSync(transcript, assistant(`live${i}`, `Live delta ${i} — ${filler(1000 + i)}`));
    await expect(page.locator('.conv').getByText(`Live delta ${i} —`).first()).toBeVisible({ timeout: 10_000 });
    await page.waitForTimeout(700);
  }
  const perDelta = (await renders(page)) - before;
  expect(perDelta, `${perDelta} markdown renders for ${DELTAS} deltas`).toBeLessThanOrEqual(DELTAS);

  const costs = await page.evaluate(() => {
    const w = window as unknown as { __frames: number[]; __muts: number[] };
    // One cost per burst of mutations (a delta): the frame interval that
    // contains the burst's first mutation, minus one 60 Hz frame.
    const bursts: number[] = [];
    for (const m of w.__muts) if (!bursts.length || m - bursts[bursts.length - 1] > 300) bursts.push(m);
    return bursts.map((m) => {
      const f = w.__frames;
      let i = f.findIndex((t) => t > m);
      if (i <= 0) i = f.length - 1;
      return Math.max(0, f[i] - f[i - 1] - 1000 / 60);
    });
  });
  costs.sort((a, b) => a - b);
  const p95 = costs[Math.min(costs.length - 1, Math.floor(costs.length * 0.95))] ?? 0;
  expect(costs.length).toBeGreaterThanOrEqual(DELTAS * 0.8);
  expect(p95, `per-delta main-thread p95 ${p95.toFixed(1)} ms (${costs.map((c) => c.toFixed(0)).join(',')})`).toBeLessThan(5);
});

test('a 70 KB tool result arrives trimmed (no page re-fetch) and loads in full on expand', async ({ page }) => {
  await page.waitForTimeout(2000);
  const refetches: string[] = [];
  const toolGets: string[] = [];
  page.on('request', (r) => {
    const u = new URL(r.url());
    if (r.method() !== 'GET') return;
    if (u.pathname.endsWith(`/sessions/${session}/transcript`)) refetches.push(u.search);
    if (u.pathname.includes(`/sessions/${session}/transcript/tool/`)) toolGets.push(u.pathname);
  });
  const big = Array.from({ length: 1400 }, (_, i) => `line ${String(i).padStart(5, '0')} ${'x'.repeat(40)}`).join('\n');
  expect(big.length).toBeGreaterThan(70_000);
  appendFileSync(transcript, toolUse('bigcall', 'toolu_big70k'));
  appendFileSync(transcript, toolResult('bigres', 'toolu_big70k', big));
  const step = page.locator('.conv .step[data-tool="shell"]').last();
  await expect(step).toBeVisible({ timeout: 15_000 });
  await expect(step).toHaveAttribute('data-status', 'ok', { timeout: 15_000 });
  // Let a would-be re-fetch fire (the old path refetched on the same tick).
  await page.waitForTimeout(1500);
  expect(refetches, 'the oversize delta must not re-fetch the page').toEqual([]);

  await step.locator('.step-row').click();
  await expect(step.locator('.step-body')).toBeVisible();
  await expect.poll(() => toolGets.length, { timeout: 10_000 }).toBe(1);
  // The full output (not the ~78-line 4 KB preview) is on screen: only a
  // result over 400 lines renders through the windowed list.
  await expect(step.locator('.out-vlist')).toBeVisible({ timeout: 10_000 });
  await expect(step.locator('.load-err')).toHaveCount(0);
  // Collapsing and re-expanding is served from the client cache.
  await step.locator('.step-row').click();
  await step.locator('.step-row').click();
  await page.waitForTimeout(300);
  expect(toolGets).toHaveLength(1);
});
