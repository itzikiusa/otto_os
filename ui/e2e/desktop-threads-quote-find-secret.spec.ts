import { test, expect, type APIRequestContext, type Page } from '@playwright/test';
import { mkdtempSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { apiCtx } from './seed';

// ─────────────────────────────────────────────────────────────────────────────
// Threads, T3 Code parity (docs/features/threads.md): quote a selected passage
// of an answer into the composer (#33124), ⌘F over a thread longer than its
// mounted window (#10439), and the private secret request card (#15907).
// The thread's history is a synthetic snapshot (600 messages), so no agent
// runs; the secret request is the daemon's list + answer routes, mocked.
// ─────────────────────────────────────────────────────────────────────────────

let ctx: APIRequestContext;
let base = '';
let wsId = '';
let dir = '';

test.beforeAll(async () => {
  const c = await apiCtx();
  ctx = c.ctx;
  base = c.base;
  dir = mkdtempSync(join(tmpdir(), 'otto-e2e-threads-qfs-'));
  const r = await ctx.post(`${base}/api/v1/workspaces`, { data: { name: 'Quote Find WS', root_path: dir } });
  expect(r.ok(), await r.text()).toBeTruthy();
  wsId = (await r.json()).id as string;
});

test.afterAll(async () => {
  if (ctx && wsId) {
    const rows = (await (await ctx.get(`${base}/api/v1/workspaces/${wsId}/runtime-sessions`)).json()) as { id: string }[];
    for (const s of rows) await ctx.post(`${base}/api/v1/runtime-sessions/${s.id}/close`).catch(() => {});
  }
  await ctx?.dispose();
  if (dir) rmSync(dir, { recursive: true, force: true });
});

test.beforeEach(async ({ page }, info) => {
  test.skip(info.project.name !== 'desktop-browser', 'desktop-browser project only');
  await page.addInitScript((id) => {
    localStorage.setItem('otto_workspace', id as string);
  }, wsId);
});

/** A thread whose history is `turns` question/answer pairs; the oldest
 *  answer holds the word "zanzibar". */
async function longThread(page: Page, turns: number): Promise<string> {
  const created = await ctx.post(`${base}/api/v1/workspaces/${wsId}/runtime-sessions`, {
    data: { provider: 'codex', cwd: dir, title: 'Long thread' },
  });
  expect(created.ok(), await created.text()).toBeTruthy();
  const id = (await created.json()).id as string;
  const at = (s: number) => new Date(Date.parse('2026-11-16T09:00:00Z') + s * 1000).toISOString();
  const row = (o: Record<string, unknown>, s: number) => ({ status: 'completed', title: null, started_at: at(s), completed_at: at(s), updated_at: at(s), ...o });
  const turnList = [];
  const items = [];
  for (let n = 0; n < turns; n++) {
    turnList.push({ id: `t${n}`, ordinal: n + 1, status: 'completed', started_at: at(n * 10), completed_at: at(n * 10 + 5) });
    items.push(row({ id: `u${n}`, turn_id: `t${n}`, ordinal: n * 2 + 1, type: 'user_message', text: `question ${n}` }, n * 10));
    const text = n === 0 ? 'The first answer: the release ships from zanzibar on Friday.' : `answer ${n}`;
    items.push(row({ id: `a${n}`, turn_id: `t${n}`, ordinal: n * 2 + 2, type: 'assistant_message', message_id: `m${n}`, streaming: false, text }, n * 10 + 5));
  }
  const snapshot = {
    provider_session: { status: 'ready', cwd: dir, model: null, updated_at: at(0), last_error: null },
    provider_thread: null,
    turns: turnList,
    turn_items: items,
    runtime_requests: [],
    plans: [],
  };
  await page.route(`**/api/v1/runtime-sessions/${id}/events**`, (r) =>
    r.fulfill({ json: { kind: 'snapshot', snapshot_sequence: 100_000, snapshot } }),
  );
  return id;
}

test('quote a passage, find past the mounted window, answer a secret request', async ({ page }) => {
  const id = await longThread(page, 300);
  const answers: unknown[] = [];
  await page.route('**/api/v1/secret-requests', (r) =>
    r.fulfill({ json: { items: [{ request_id: 'sr1', session_id: id, workspace_id: wsId, prompt: 'A GitHub token to push the release', name: 'GH_TOKEN' }] } }),
  );
  await page.route('**/api/v1/secret-requests/sr1', (r) => {
    answers.push(r.request().postDataJSON());
    return r.fulfill({ status: 204 });
  });
  await page.goto(`/#/threads/${id}`);
  const thread = page.getByTestId(`runtime-thread-${id}`);
  await expect(thread.getByText('answer 299')).toBeVisible({ timeout: 30_000 });
  const composer = thread.getByTestId('runtime-composer');

  // Quote: select a passage of the newest answer → Quote → a markdown quote.
  await composer.fill('About this:');
  const answer = thread.locator('.rt-assistant', { hasText: 'answer 299' });
  await answer.evaluate((el) => {
    const p = el.querySelector('p') ?? el;
    const range = document.createRange();
    range.selectNodeContents(p);
    const sel = window.getSelection()!;
    sel.removeAllRanges();
    sel.addRange(range);
  });
  await page.getByTestId('selection-quote').click();
  await expect(composer).toHaveValue('About this:\n\n> answer 299\n\n');
  await expect(composer).toBeFocused();
  await expect(page.getByTestId('selection-quote')).toHaveCount(0);

  // Find: the oldest answer is not mounted (600 rows, 200 shown), yet ⌘F
  // counts it and reveals it.
  await expect(thread.getByText('zanzibar')).toHaveCount(0);
  await page.evaluate(() => (document.activeElement as HTMLElement | null)?.blur());
  await page.keyboard.press('ControlOrMeta+f');
  const bar = page.locator('.otto-find-bar');
  await expect(bar).toBeVisible();
  await page.keyboard.type('zanzibar');
  await expect(bar.locator('.find-count')).toHaveText('1 / 1');
  await expect(thread.getByText(/zanzibar/)).toBeVisible();
  await page.keyboard.press('Escape');

  // Secret: the card shows for the owner; the value goes to the daemon only.
  const card = thread.getByTestId('runtime-secret-card');
  await expect(card).toContainText('A GitHub token to push the release');
  await card.getByTestId('runtime-secret-input').fill('ghp_e2e_secret');
  await card.getByTestId('runtime-secret-send').click();
  await expect(card).toHaveCount(0);
  expect(answers).toEqual([{ value: 'ghp_e2e_secret' }]);
  await expect(composer).toHaveValue('About this:\n\n> answer 299\n\n');
});
