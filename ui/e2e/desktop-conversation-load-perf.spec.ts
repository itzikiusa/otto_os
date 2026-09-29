import { test, expect, type APIRequestContext, type Page } from '@playwright/test';
import { appendFileSync, copyFileSync, cpSync, existsSync, mkdtempSync, readdirSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { basename, dirname, join } from 'node:path';
import { apiCtx, seedWorkspace } from './seed';
import { isDesktopProject, watchFatalUiErrors } from './perf';

// ─────────────────────────────────────────────────────────────────────────────
// Time-to-first-turns of a BIG chat (the "it takes a couple of seconds" case).
//
// A real Claude transcript (`OTTO_CONV_FIXTURE=<…/sid.jsonl>`, plus its
// `<sid>/subagents/` sidecars when present) is copied to a temp dir and a live
// shell session points at it (the `OTTO_E2E=1` `meta.e2e_transcript_path`
// hook). Measured, per scenario:
//   • click "Chat" → the first turn is in the DOM and painted (in-page clock);
//   • the `GET …/transcript` server time (requestStart → responseStart), the
//     body size and the whole response time;
//   • the longest main-thread frame gap while it loaded.
// Scenarios: cold (first read of the file), reopen (Terminal → Chat on the
// same page), reload (a new page; the daemon side is warm), and cold while the
// agent is writing (an append every 200 ms — the old "transcript busy" race).
// Skipped without a fixture: the corpus is the user's own, never committed.
// ─────────────────────────────────────────────────────────────────────────────

test.setTimeout(240_000);

const FIXTURE = process.env.OTTO_CONV_FIXTURE ?? '';
const TS = '2026-09-29T12:00:00Z';

let root: APIRequestContext;
let base = '';
let wsId = '';
let session = '';
let dir = '';
let transcript = '';
let fatal: string[] = [];

// The throwaway daemon's boot work (usage-tailer re-ingest of the real
// corpus, model catalog scrapes) competes for the CPU for several seconds;
// let it settle once so the cold numbers measure the chat, not the boot.
test.beforeAll(async () => {
  const ms = Number(process.env.OTTO_CONV_SETTLE_MS ?? '10000');
  if (FIXTURE && existsSync(FIXTURE) && ms > 0) await new Promise((r) => setTimeout(r, ms));
});

interface Sample {
  firstTurnMs: number;
  /** The GET's `responseEnd` → first painted turn: JSON parse + render. */
  renderMs: number;
  maxGapMs: number;
  serverMs: number;
  responseMs: number;
  bytes: number;
  gets: number;
}

test.beforeEach(async ({ page }, info) => {
  test.skip(!isDesktopProject(info.project.name), 'desktop projects only');
  test.skip(!FIXTURE || !existsSync(FIXTURE), 'set OTTO_CONV_FIXTURE to a real transcript');
  ({ ctx: root, base } = await apiCtx());
  wsId = await seedWorkspace(root, base);
  dir = mkdtempSync(join(tmpdir(), 'otto-conv-load-'));
  transcript = join(dir, basename(FIXTURE));
  copyFileSync(FIXTURE, transcript);
  const subs = join(dirname(FIXTURE), basename(FIXTURE, '.jsonl'), 'subagents');
  if (existsSync(subs)) cpSync(subs, join(dir, basename(FIXTURE, '.jsonl'), 'subagents'), { recursive: true });
  const created = await root.post(`${base}/api/v1/workspaces/${wsId}/sessions`, {
    data: { kind: 'agent', provider: 'shell', title: 'ConvLoad', cwd: dir, meta: { origin: 'e2e', nested_provider: 'claude', e2e_transcript_path: transcript } },
  });
  expect(created.ok(), await created.text()).toBeTruthy();
  session = (await created.json()).id as string;
  await expect.poll(async () => (await (await root.get(`${base}/api/v1/sessions/${session}`)).json()).live).toBe(true);
  fatal = watchFatalUiErrors(page);
  await page.addInitScript((id) => {
    localStorage.setItem('otto_workspace', id as string);
    localStorage.setItem('otto_firstrun_dismissed', '1');
  }, wsId);
});

test.afterEach(async () => {
  const seen = fatal;
  fatal = [];
  if (session && root) await root.delete(`${base}/api/v1/sessions/${session}`).catch(() => {});
  session = '';
  await root?.dispose();
  if (dir) rmSync(dir, { recursive: true, force: true });
  dir = '';
  expect(seen, 'no fatal UI error during the run').toEqual([]);
});

async function openTerminalPane(page: Page): Promise<void> {
  await page.goto(`/#/agents/${session}`);
  await expect(page.locator('.pane-head', { hasText: 'ConvLoad' }).first()).toBeVisible({ timeout: 20_000 });
  const term = page.locator('.view-seg button', { hasText: 'Terminal' });
  if (await term.count()) await term.first().click();
}

/** Click "Chat" in-page and time it to the first painted turn. */
async function openChat(page: Page): Promise<Sample> {
  const gets: { server: number; total: number; bytes: number }[] = [];
  const onDone = async (r: import('@playwright/test').Request) => {
    const u = new URL(r.url());
    if (r.method() !== 'GET' || !u.pathname.endsWith(`/sessions/${session}/transcript`)) return;
    const tm = r.timing();
    const resp = await r.response();
    const body = resp ? await resp.body().catch(() => Buffer.alloc(0)) : Buffer.alloc(0);
    gets.push({ server: tm.responseStart - tm.requestStart, total: tm.responseEnd, bytes: body.length });
  };
  page.on('requestfinished', onDone);
  const r = await page.evaluate(
    (sid) =>
      new Promise<{ firstTurnMs: number; renderMs: number; maxGapMs: number }>((resolve) => {
        const btn = [...document.querySelectorAll<HTMLButtonElement>('.view-seg button')].find((b) => b.textContent?.includes('Chat'));
        // Dev mode loads hundreds of modules: a full timing buffer drops new entries.
        performance.clearResourceTimings();
        const t0 = performance.now();
        let last = t0;
        let maxGap = 0;
        btn?.click();
        // `responseEnd` survives cross-origin resource timing (no TAO needed).
        const responseEnd = (): number => {
          const e = performance
            .getEntriesByType('resource')
            .filter((x) => x.name.includes(`/sessions/${sid}/transcript`) && !x.name.includes('/touch') && x.startTime >= t0);
          return e.length ? (e[e.length - 1] as PerformanceResourceTiming).responseEnd : NaN;
        };
        const tickFrame = (t: number): void => {
          maxGap = Math.max(maxGap, t - last);
          last = t;
          if (document.querySelector('.conv[data-loaded="true"] [data-turn-id]')) {
            // One more frame so the measurement ends after the paint.
            requestAnimationFrame((t2) =>
              resolve({ firstTurnMs: t2 - t0, renderMs: t2 - responseEnd(), maxGapMs: Math.max(maxGap, t2 - t) }),
            );
            return;
          }
          if (t - t0 > 60_000) return resolve({ firstTurnMs: -1, renderMs: NaN, maxGapMs: maxGap });
          requestAnimationFrame(tickFrame);
        };
        requestAnimationFrame(tickFrame);
      }),
    session,
  );
  await page.waitForTimeout(300);
  page.off('requestfinished', onDone);
  const g = gets[gets.length - 1] ?? { server: -1, total: -1, bytes: 0 };
  return { ...r, serverMs: g.server, responseMs: g.total, bytes: g.bytes, gets: gets.length };
}

function log(label: string, s: Sample): void {
  console.log(
    `[conv-load] ${test.info().project.name} ${label}: first turn ${s.firstTurnMs.toFixed(0)} ms (response→painted ${s.renderMs.toFixed(0)} ms), max frame gap ${s.maxGapMs.toFixed(0)} ms, GET server ${s.serverMs.toFixed(0)} ms / response ${s.responseMs.toFixed(0)} ms, ${(s.bytes / 1024).toFixed(0)} KB, ${s.gets} GET(s)`,
  );
}

test('big transcript: cold open, reopen, reload', async ({ page }) => {
  await openTerminalPane(page);
  const cold = await openChat(page);
  log('cold', cold);
  expect(cold.firstTurnMs).toBeGreaterThan(0);

  // Reopen on the same page (the store may still hold the page).
  await page.locator('.view-seg button', { hasText: 'Terminal' }).first().click();
  await page.waitForTimeout(500);
  const reopen = await openChat(page);
  log('reopen', reopen);

  // A new page: the client is cold, the daemon holds the fold (tail / cache).
  await openTerminalPane(page);
  await page.waitForTimeout(500);
  const reload = await openChat(page);
  log('reload', reload);
  expect(reload.firstTurnMs).toBeGreaterThan(0);
});

test('big transcript: cold open while the agent is writing', async ({ page }) => {
  await openTerminalPane(page);
  let i = 0;
  const writer = setInterval(() => {
    appendFileSync(
      transcript,
      JSON.stringify({ type: 'system', subtype: 'turn_duration', durationMs: 1, uuid: `w${i++}`, timestamp: TS }) + '\n',
    );
  }, 200);
  try {
    const s = await openChat(page);
    log('cold+writing', s);
    expect(s.firstTurnMs).toBeGreaterThan(0);
  } finally {
    clearInterval(writer);
  }
  // Sanity: the subagent sidecars were copied when the fixture had them.
  const subs = join(dir, basename(FIXTURE, '.jsonl'), 'subagents');
  if (existsSync(subs)) expect(readdirSync(subs).length).toBeGreaterThan(0);
});
