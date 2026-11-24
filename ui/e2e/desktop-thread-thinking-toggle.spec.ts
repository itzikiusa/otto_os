import { test, expect, type APIRequestContext, type Locator, type Page } from '@playwright/test';
import { mkdtempSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { apiCtx } from './seed';

// The agent's "Thinking" row obeys the person (0.5.1 owner report: "I cannot
// close it … on occasions it closes it automatically, and on occasions opens
// it, on its own"). A click always wins: a row someone opened stays open
// through more deltas, the next steps and the end of the turn; one they closed
// stays closed; an untouched row still clamps while it streams and folds into
// "Worked for …" when the turn ends. The fakes stream no reasoning, so the
// thread's /events answer is a scripted snapshot that grows one delta per
// step (new item objects each time, as live upserts are), and a window
// "focus" (the runtime view's wake) replays it. Run on Threads and on an
// Agents chat pane (the same RuntimeThread timeline).

let ctx: APIRequestContext;
let base = '';
let wsId = '';
let dir = '';
const made: string[] = [];

test.beforeAll(async () => {
  ({ ctx, base } = await apiCtx());
  dir = mkdtempSync(join(tmpdir(), 'otto-e2e-thread-think-'));
  const w = await ctx.post(`${base}/api/v1/workspaces`, { data: { name: 'Thinking Toggle WS', root_path: dir } });
  expect(w.ok(), await w.text()).toBeTruthy();
  wsId = (await w.json()).id as string;
});

test.afterAll(async () => {
  for (const id of made) await ctx.delete(`${base}/api/v1/runtime-sessions/${id}`).catch(() => {});
  await ctx?.dispose();
  if (dir) rmSync(dir, { recursive: true, force: true });
});

test.beforeEach(async ({ page }, info) => {
  test.skip(info.project.name !== 'desktop-browser', 'desktop-browser project only');
  test.setTimeout(120_000);
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.addInitScript((id) => {
    if (sessionStorage.getItem('thread-think-init')) return;
    sessionStorage.setItem('thread-think-init', '1');
    localStorage.setItem('otto_workspace', id as string);
    localStorage.setItem('otto_firstrun_dismissed', '1');
    localStorage.setItem('otto_nav_all_ws', '0');
    localStorage.setItem('otto_view_mode', 'tabs');
  }, wsId);
});

const T0 = Date.parse('2026-11-24T10:00:00Z');
const at = (s: number) => new Date(T0 + s * 1000).toISOString();
const LAST = 26;
const thought = (name: string, from: number, k: number) =>
  [`${name} plan line A`, `${name} plan line B`, `${name} plan line C`, `${name} plan line D`]
    .concat(Array.from({ length: Math.max(0, k - from) }, (_, i) => `${name} delta ${i + 1}`))
    .join('\n');

/** The thread at step `k`: turn 1 thinks (r1, 0–11), runs c1 (12), thinks
 *  again (r2, 13–18), runs c2 (19), answers (20) and ends (21); turn 2 thinks
 *  (r3, 22–25) and ends (26). */
function snapshot(k: number) {
  const items: Record<string, unknown>[] = [];
  let ord = 0;
  const base = (id: string, turn: string, s: number, done: number | null) => ({
    id,
    turn_id: turn,
    ordinal: ++ord,
    status: done === null ? 'running' : 'completed',
    title: null,
    started_at: at(s),
    completed_at: done === null ? null : at(done),
    updated_at: at(done ?? s + k),
  });
  const reasoning = (id: string, turn: string, from: number, to: number) => {
    if (k < from) return;
    const done = k > to;
    items.push({ ...base(id, turn, from, done ? to + 1 : null), type: 'reasoning', text: thought(id, from, Math.min(k, to)), streaming: !done });
  };
  const command = (id: string, turn: string, s: number) => {
    if (k < s) return;
    items.push({ ...base(id, turn, s, k > s ? s + 1 : null), type: 'command_execution', input: `rg ${id} src`, ...(k > s ? { output: 'ok\n', exit_code: 0 } : {}) });
  };
  const answer = (id: string, turn: string, s: number) => {
    if (k < s) return;
    items.push({ ...base(id, turn, s, k > s ? s + 1 : null), type: 'assistant_message', message_id: `m-${id}`, text: `Answer ${id}`, streaming: k <= s });
  };
  items.push({ ...base('u1', 't1', 0, 0), type: 'user_message', text: 'think about it' });
  reasoning('r1', 't1', 0, 11);
  command('c1', 't1', 12);
  reasoning('r2', 't1', 13, 18);
  command('c2', 't1', 19);
  answer('a1', 't1', 20);
  const turns = [{ id: 't1', ordinal: 1, status: k >= 21 ? 'completed' : 'running', started_at: at(0), completed_at: k >= 21 ? at(21) : null, origin: 'user' }];
  if (k >= 22) {
    items.push({ ...base('u2', 't2', 22, 22), type: 'user_message', text: 'and again' });
    reasoning('r3', 't2', 22, 25);
    answer('a2', 't2', 26);
    turns.push({ id: 't2', ordinal: 2, status: k >= 26 ? 'completed' : 'running', started_at: at(22), completed_at: k >= 26 ? at(26) : null, origin: 'user' });
  }
  const live = turns.some((t) => t.status === 'running');
  return {
    kind: 'snapshot',
    snapshot_sequence: 1000 + k,
    snapshot: {
      provider_session: { status: live ? 'running' : 'ready', cwd: dir, model: null, updated_at: at(k), last_error: null },
      provider_thread: null,
      turns,
      turn_items: items,
      runtime_requests: [],
      plans: [],
    },
  };
}

/** A scripted thread: `to(k)` moves it to step k and waits for the UI to read it. */
async function scripted(page: Page, title: string) {
  const r = await ctx.post(`${base}/api/v1/workspaces/${wsId}/runtime-sessions`, { data: { provider: 'codex', cwd: dir, title } });
  expect(r.ok(), await r.text()).toBeTruthy();
  const id = (await r.json()).id as string;
  made.push(id);
  let step = 0;
  await page.route(`**/api/v1/runtime-sessions/${id}/events**`, (route) => route.fulfill({ json: snapshot(step) }));
  const to = async (k: number, _thread: Locator) => {
    for (step = step + 1; step <= k; step++) {
      const read = page.waitForResponse((res) => res.url().includes(`/runtime-sessions/${id}/events`));
      await page.evaluate(() => window.dispatchEvent(new Event('focus')));
      await read;
      await page.waitForTimeout(60); // ui-guards: allow — one delta's render before the next
    }
    step = k;
  };
  return { id, to };
}

const think = (thread: Locator, id: string) => thread.getByTestId(`runtime-item-${id}`);
const more = (row: Locator) => row.locator('.rw-more');

for (const host of ['threads', 'agents'] as const) {
  const open = async (page: Page, id: string): Promise<Locator> => {
    await page.goto(host === 'threads' ? `/#/threads/${id}` : `/#/agents/${id}`);
    const root = host === 'threads' ? page : page.getByTestId(`thread-pane-${id}`);
    const thread = root.getByTestId(`runtime-thread-${id}`).first();
    await expect(thread).toBeVisible({ timeout: 30_000 });
    await expect(think(thread, 'r1')).toBeVisible({ timeout: 15_000 });
    return thread;
  };

  test(`${host}: an opened Thinking row stays open through deltas, the next steps and the end of the turn`, async ({ page }) => {
    const t = await scripted(page, `Opened thinking (${host})`);
    const thread = await open(page, t.id);
    await t.to(2, thread);
    await more(think(thread, 'r1')).click();
    await expect(more(think(thread, 'r1'))).toHaveAttribute('aria-expanded', 'true');
    await t.to(12, thread); // 10 more deltas, then r1 ends
    await expect(more(think(thread, 'r1'))).toHaveAttribute('aria-expanded', 'true');
    await expect(think(thread, 'r1').locator('.rw-think')).toContainText('r1 delta 11');
    await t.to(19, thread); // the next steps (c1, r2, c2)
    await expect(more(think(thread, 'r1'))).toHaveAttribute('aria-expanded', 'true');
    await expect(think(thread, 'r1').locator('.rw-think')).toBeVisible();
    await t.to(21, thread); // the turn ends
    await expect(more(think(thread, 'r1'))).toHaveAttribute('aria-expanded', 'true');
    await expect(think(thread, 'r1').locator('.rw-think')).toBeVisible();
    await expect(think(thread, 'r1').locator('.rw-think')).toContainText('r1 delta 11');
    await t.to(LAST, thread); // a following turn
    await expect(more(think(thread, 'r1'))).toHaveAttribute('aria-expanded', 'true');
  });

  test(`${host}: a Thinking row closed while it streams stays closed through deltas and the next turn`, async ({ page }) => {
    const t = await scripted(page, `Closed thinking (${host})`);
    const thread = await open(page, t.id);
    await t.to(2, thread);
    await more(think(thread, 'r1')).click();
    await t.to(4, thread);
    await more(think(thread, 'r1')).click(); // closed while still thinking
    await expect(more(think(thread, 'r1'))).toHaveAttribute('aria-expanded', 'false');
    for (const k of [8, 11, 12, 16, 19, 20, LAST]) {
      await t.to(k, thread);
      const row = think(thread, 'r1');
      // Closed: clamped where it shows, or folded away with its finished stretch.
      if (await row.count()) {
        await expect(more(row), `step ${k}`).toHaveAttribute('aria-expanded', 'false');
        await expect(row.locator('.rw-think'), `step ${k}`).not.toContainText('r1 delta 6');
      }
    }
  });

  test(`${host}: an untouched Thinking row clamps while it streams and folds when the turn ends`, async ({ page }) => {
    const t = await scripted(page, `Untouched thinking (${host})`);
    const thread = await open(page, t.id);
    await t.to(16, thread);
    await expect(more(think(thread, 'r2'))).toHaveAttribute('aria-expanded', 'false');
    await expect(think(thread, 'r2').locator('.rw-think')).not.toContainText('r2 delta 3');
    // The finished steps before the live one fold into "N earlier steps" and
    // stay folded while the turn runs — also in the pause where nothing runs
    // (c2 done, the answer starting): they never pop back out on their own.
    await t.to(19, thread);
    await expect(thread.getByTestId('runtime-earlier-steps')).toHaveCount(1);
    await expect(think(thread, 'r1')).toBeHidden();
    await t.to(20, thread);
    await expect(thread.getByTestId('runtime-earlier-steps')).toHaveCount(1);
    await expect(think(thread, 'r1')).toBeHidden();
    await t.to(21, thread);
    await expect(thread.getByTestId('runtime-fold-r1')).toBeVisible();
    await expect(think(thread, 'r2')).toHaveCount(0);
  });
}
