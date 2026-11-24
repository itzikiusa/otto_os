// A long Claude Code session for the long-chat specs — the JS twin of
// crates/otto-transcript/tests/support/long_session.rs, shaped like the
// owner's daily sessions (sizes and record types only): 60+ MB, 20k+ records,
// ~1 in 25 exchanges a multi-hundred-KB tool dump duplicated in
// toolUseResult, base64 screenshots, CLI sidecar/telemetry lines,
// compactions, and a final turn still running (tool calls, no answer yet).
// Fed to a seeded session through the OTTO_E2E=1 meta.e2e_transcript_path hook.
import { appendFileSync, closeSync, openSync, writeSync } from 'node:fs';
import { join } from 'node:path';
import { expect, type APIRequestContext, type Page } from '@playwright/test';
import { tempDir } from './seed';

export const LONG_SID = '5e55e55e-0000-4000-8000-0000000000e2';

export interface LongChat { dir: string; path: string; bytes: number; records: number; exchanges: number; lastPrompt: string }

export function writeLongChat(minBytes = 60 << 20, minRecords = 20_000): LongChat {
  const dir = tempDir('otto-long-chat-');
  const path = join(dir, `${LONG_SID}.jsonl`);
  const fd = openSync(path, 'w');
  let bytes = 0;
  let records = 0;
  const put = (v: unknown) => {
    const line = JSON.stringify(v) + '\n';
    bytes += writeSync(fd, line);
    records++;
  };
  const dump = 'lorem ipsum dolor sit amet 0123456789 '.repeat(8 * 1024);
  const small = 'ok: line of tool output\n'.repeat(80);
  const png = 'iVBORw0KGgo'.repeat(14 * 1024);
  const meta = (n: number, k: number) => ({ sessionId: LONG_SID, cwd: dir, timestamp: new Date(Date.UTC(2026, 10, 1) + n * 60_000 + k * 1000).toISOString() });
  const usage = { input_tokens: 3, output_tokens: 120, cache_read_input_tokens: 91_000 };
  const asst = (n: number, k: string, content: unknown[]) => put({ type: 'assistant', uuid: `a-${n}-${k}`, ...meta(n, 1), requestId: `req-${n}-${k}`, message: { id: `msg-${n}-${k}`, role: 'assistant', model: 'claude-opus-5-5', content, usage } });
  const result = (n: number, k: string, id: string, content: unknown, extra?: unknown) => put({ type: 'user', uuid: `r-${n}-${k}`, ...meta(n, 2), message: { role: 'user', content: [{ tool_use_id: id, type: 'tool_result', content }] }, ...(extra ? { toolUseResult: extra } : {}) });
  let n = 0;
  while (bytes < minBytes || records < minRecords) {
    put({ type: 'user', uuid: `u-${n}`, ...meta(n, 0), message: { role: 'user', content: `prompt ${n}: look at the next failing check and fix it` } });
    put({ type: 'permission-mode', permissionMode: 'bypassPermissions', sessionId: LONG_SID });
    put({ type: 'last-prompt', lastPrompt: `prompt ${n}`, sessionId: LONG_SID });
    put({ type: 'api-request', requestId: `req-${n}-0`, model: 'claude-opus-5-5', sessionId: LONG_SID });
    asst(n, 't', [{ type: 'thinking', thinking: '', signature: 'sig' }]);
    for (let k = 0; k < 3; k++) {
      const id = `toolu-${n}-${k}`;
      asst(n, String(k), [{ type: 'tool_use', id, name: 'Bash', input: { command: `cargo test -p crate-${k} -- case_${n}`, description: 'run the case' } }]);
      if (n % 25 === 11 && k === 2) result(n, String(k), id, [{ type: 'image', source: { type: 'base64', media_type: 'image/png', data: png + n } }]);
      else {
        const text = n % 25 === 3 && k === 1 ? dump : small;
        result(n, String(k), id, text, { stdout: text, stderr: '', interrupted: false });
      }
    }
    asst(n, 'x', [{ type: 'text', text: `Done with step ${n}: the check passes now.` }]);
    put({ type: 'ai-title', aiTitle: 'Long running session', sessionId: LONG_SID });
    if (n % 500 === 499) {
      put({ type: 'system', subtype: 'compact_boundary', content: 'Conversation compacted', uuid: `c-${n}`, ...meta(n, 3) });
      put({ type: 'user', uuid: `cs-${n}`, ...meta(n, 3), isCompactSummary: true, message: { role: 'user', content: `This session is being continued from a previous conversation. Summary: steps 0..${n} fixed.` } });
    }
    n++;
  }
  const lastPrompt = `prompt ${n}: final sweep, still running`;
  put({ type: 'user', uuid: `u-${n}`, ...meta(n, 0), message: { role: 'user', content: lastPrompt } });
  for (let k = 0; k < 120; k++) {
    const id = k === 60 ? `toolu-${n}-big` : `toolu-${n}-r${k}`;
    asst(n, `r${k}`, [{ type: 'tool_use', id, name: 'Read', input: { file_path: `${dir}/src/file_${k}.rs` } }]);
    result(n, `r${k}`, id, k === 60 ? dump.repeat(7) : small);
  }
  closeSync(fd);
  return { dir, path, bytes, records, exchanges: n, lastPrompt };
}

/** A plain chat of `turns` prompt/answer exchanges (a few screens tall, no tool dumps). */
export function writeChat(turns = 40): LongChat {
  const dir = tempDir('otto-chat-');
  const path = join(dir, `${LONG_SID}.jsonl`);
  const at = (n: number, k: number) => new Date(Date.UTC(2026, 10, 1) + n * 60_000 + k * 1000).toISOString();
  const lines: string[] = [];
  for (let n = 0; n < turns; n++) {
    lines.push(JSON.stringify({ type: 'user', uuid: `u-${n}`, sessionId: LONG_SID, cwd: dir, timestamp: at(n, 0), message: { role: 'user', content: `prompt ${n}: look at the next failing check` } }));
    lines.push(JSON.stringify({ type: 'assistant', uuid: `a-${n}`, sessionId: LONG_SID, cwd: dir, timestamp: at(n, 1), requestId: `req-${n}`, message: { id: `msg-${n}`, role: 'assistant', model: 'claude-opus-5-5', content: [{ type: 'text', text: `Step ${n} is fixed.\n\nThe check passes now; the next one is below.` }] } }));
  }
  appendFileSync(path, lines.join('\n') + '\n');
  return { dir, path, bytes: 0, records: lines.length, exchanges: turns, lastPrompt: `prompt ${turns - 1}: look at the next failing check` };
}

/** Seed a live (inert shell) session whose Chat reads `chat.path`. */
export async function seedLongChat(ctx: APIRequestContext, base: string, wsId: string, chat: LongChat): Promise<string> {
  const r = await ctx.post(`${base}/api/v1/workspaces/${wsId}/sessions`, {
    data: { kind: 'agent', provider: 'shell', title: 'Long day', cwd: chat.dir, meta: { origin: 'e2e', nested_provider: 'claude', e2e_transcript_path: chat.path } },
  });
  expect(r.ok(), await r.text()).toBeTruthy();
  return (await r.json()).id as string;
}

export interface ChatMetrics { project: string; fileBytes: number; records: number; firstPaintMs: number; firstPageMs: number; firstPageBytes: number; earlierPages: number; earlierBytes: number; rePaintMs: number | null }

/** Open the session's Chat on `page` (Terminal ⇄ Chat), check the newest
 *  turns, page older ones in by scrolling up, take a live append without a
 *  re-read, and send a follow-up from the composer. Returns what it measured. */
export async function exerciseLongChat(page: Page, id: string, chat: LongChat, project: string, phone = false): Promise<ChatMetrics> {
  const pages: { url: string; bytes: number; ms: number }[] = [];
  page.on('response', async (resp) => {
    const u = new URL(resp.url());
    if (u.pathname.endsWith(`/sessions/${id}/transcript`)) pages.push({ url: u.search, bytes: (await resp.body().catch(() => Buffer.alloc(0))).length, ms: Math.round(resp.request().timing().responseEnd) });
  });
  const pane = page.locator(`[data-pane-key][data-session="${id}"]`).first();
  let t0 = 0;
  if (phone) {
    // A phone-width pane hides the Terminal ⇄ Chat switch; it opens on the
    // view remembered for the session (the switch's own per-session key).
    await page.addInitScript((sid) => localStorage.setItem(`otto_pane_view:${sid}`, 'chat'), id);
    await page.goto(`/#/agents/${id}`);
  } else {
    await page.goto(`/#/agents/${id}`);
    await expect(pane.locator('.term-host')).toBeVisible({ timeout: 30_000 });
    t0 = await page.evaluate(() => performance.now());
    await pane.getByTestId('pane-view').getByRole('button', { name: 'Chat' }).click();
  }
  const list = pane.locator('.conv-list');
  await list.locator('article.turn').last().waitFor({ state: 'visible', timeout: 30_000 });
  const firstPaintMs = await page.evaluate(async (start) => {
    await new Promise((r) => requestAnimationFrame(() => requestAnimationFrame(r)));
    return performance.now() - start;
  }, t0);
  // The newest turns: the still-running final turn's 60 Read calls, no error.
  await expect(pane.locator('.conv')).toContainText(/Read 60 files/);
  await expect(pane.locator('.conv')).not.toContainText(/internal server error|couldn.t load/i);
  await expect.poll(() => pages.length).toBeGreaterThan(0);
  const firstPageBytes = pages[0].bytes;
  const firstPageMs = pages[0].ms;

  // Scroll up: older turns load in front, the reading position is kept.
  for (let i = 0; i < 8 && !(await list.getByText(chat.lastPrompt).count()); i++) {
    // One run of tool calls folds into one item, so count the pages read.
    const before = pages.filter((p) => p.url.includes('before=')).length;
    // Scrolling up to the top loads more; a list too short to scroll shows the
    // "Load earlier messages" row instead.
    if (await list.evaluate((el) => el.scrollHeight > el.clientHeight + 4)) await list.evaluate((el) => el.scrollTo({ top: 0 }));
    else await list.getByRole('button', { name: 'Load earlier messages' }).click();
    await expect.poll(() => pages.filter((p) => p.url.includes('before=')).length, { timeout: 20_000 }).toBeGreaterThan(before);
    await expect(list.getByText('Loading earlier messages…')).toHaveCount(0, { timeout: 20_000 });
  }
  await expect(list.getByText(chat.lastPrompt)).toHaveCount(1);
  const earlier = pages.filter((p) => p.url.includes('before='));
  expect(earlier.length).toBeGreaterThan(0);

  // Live: a turn the agent appends lands without re-reading the page.
  const reads = pages.length;
  await list.evaluate((el) => el.scrollTo({ top: el.scrollHeight }));
  appendFileSync(chat.path, JSON.stringify({ type: 'assistant', uuid: 'a-live', sessionId: LONG_SID, timestamp: new Date().toISOString(), requestId: 'req-live', message: { id: 'msg-live', role: 'assistant', model: 'claude-opus-5-5', content: [{ type: 'text', text: 'Live answer appended while you watch.' }] } }) + '\n');
  await expect(pane.locator('.conv')).toContainText('Live answer appended while you watch.', { timeout: 20_000 });
  expect(pages.length, 'the live turn came over the socket, not a page re-read').toBe(reads);

  // A follow-up from the composer reaches the session.
  const sent = page.waitForRequest((r) => r.method() === 'POST' && r.url().endsWith(`/sessions/${id}/input`));
  const box = pane.getByRole('textbox', { name: /^Message/ });
  await box.fill('follow-up from the long chat');
  await box.press('Enter');
  expect((await sent).postDataJSON()).toMatchObject({ text: 'follow-up from the long chat', submit: true });

  // Terminal → Chat again (the switch the owner makes all day): time to paint.
  let rePaintMs: number | null = null;
  if (!phone) {
    await pane.getByTestId('pane-view').getByRole('button', { name: 'Terminal' }).click();
    await expect(pane.locator('.term-host')).toBeVisible();
    const t1 = await page.evaluate(() => performance.now());
    await pane.getByTestId('pane-view').getByRole('button', { name: 'Chat' }).click();
    await list.locator('article.turn').last().waitFor({ state: 'visible' });
    rePaintMs = Math.round(await page.evaluate(async (start) => {
      await new Promise((r) => requestAnimationFrame(() => requestAnimationFrame(r)));
      return performance.now() - start;
    }, t1));
  }
  return { project, fileBytes: chat.bytes, records: chat.records, firstPaintMs: Math.round(firstPaintMs), firstPageMs, firstPageBytes, rePaintMs, earlierPages: earlier.length, earlierBytes: earlier.reduce((n, p) => n + p.bytes, 0) };
}
