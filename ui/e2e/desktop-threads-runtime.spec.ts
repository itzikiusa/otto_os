import { test, expect, type APIRequestContext, type Page } from '@playwright/test';
import { mkdtempSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { apiCtx } from './seed';
import { expectFullyInViewport, expectNoHorizontalOverflow } from './helpers';
import type { Locator } from '@playwright/test';

/** Relative luminance (0 dark … 1 light) of an element's computed background. */
async function bgLuminance(el: Locator): Promise<number> {
  return el.evaluate((node) => {
    const [r, g, b] = (getComputedStyle(node).backgroundColor.match(/[\d.]+/g) ?? ['0', '0', '0']).map(Number);
    return (0.2126 * r + 0.7152 * g + 0.0722 * b) / 255;
  });
}

// ─────────────────────────────────────────────────────────────────────────────
// Threads on the structured agent runtime (docs/features/threads-structured-runtime.md):
// a new Codex thread runs `codex app-server` — in the isolated daemon that is
// otto-agent-protocol's `otto-fake-codex` (global-setup's codex shim execs it
// for `app-server`, so it needs OTTO_E2E_BIN pointing at a build that has it
// next to ottod). The fake answers by prompt: echo, `ask` (a command approval)
// and `hang` (runs until interrupted).
// ─────────────────────────────────────────────────────────────────────────────

let ctx: APIRequestContext;
let base = '';
let wsId = '';
let fixtureDir = '';

test.beforeAll(async () => {
  const c = await apiCtx();
  ctx = c.ctx;
  base = c.base;
  fixtureDir = mkdtempSync(join(tmpdir(), 'otto-e2e-threads-rt-'));
  writeFileSync(join(fixtureDir, 'README.md'), '# Runtime threads\n');
  const r = await ctx.post(`${base}/api/v1/workspaces`, { data: { name: 'Runtime Threads WS', root_path: fixtureDir } });
  expect(r.ok(), await r.text()).toBeTruthy();
  wsId = (await r.json()).id as string;
});

test.afterAll(async () => {
  if (ctx && wsId) {
    const rows = (await (await ctx.get(`${base}/api/v1/workspaces/${wsId}/runtime-sessions`)).json()) as { id: string }[];
    for (const s of rows) await ctx.post(`${base}/api/v1/runtime-sessions/${s.id}/close`).catch(() => {});
  }
  await ctx?.dispose();
  if (fixtureDir) rmSync(fixtureDir, { recursive: true, force: true });
});

test.beforeEach(async ({ page }, info) => {
  test.skip(info.project.name !== 'desktop-browser', 'desktop-browser project only');
  await page.addInitScript((id) => {
    if (sessionStorage.getItem('threads-rt-init')) return;
    sessionStorage.setItem('threads-rt-init', '1');
    localStorage.setItem('otto_workspace', id as string);
    localStorage.removeItem('otto_threads_v1');
  }, wsId);
});

test('a Codex runtime thread streams, asks inline, queues a follow-up and stops', async ({ page }, info) => {
  await page.goto('/#/threads');
  // Other specs' threads (any workspace) may open first; start a new one.
  await page.getByTestId('threads-new-btn').click();
  const stage = page.getByTestId('new-thread-stage');
  await expect(stage).toBeVisible({ timeout: 30_000 });

  // Pick Codex in the one agent panel; runtime threads have no access-mode switch.
  await stage.getByTestId('picker-agent').click();
  const picker = page.getByTestId('model-picker');
  await picker.getByTestId('model-picker-provider-codex').click();
  await expect(picker.getByTestId('picker-mode-pop')).toHaveCount(0);
  await page.keyboard.press('Escape');
  await expect(stage.getByTestId('picker-mode')).toHaveCount(0);

  await stage.getByTestId('new-thread-input').fill('hello runtime');
  await stage.getByTestId('new-thread-send-btn').click();

  const thread = page.locator('[data-testid^="runtime-thread-"]');
  await expect(thread).toBeVisible({ timeout: 30_000 });
  await expect(thread.locator('.rt-bubble', { hasText: 'hello runtime' })).toBeVisible();
  await expect(thread.getByText('echo: hello runtime')).toBeVisible({ timeout: 15_000 });
  // The placeholder title drops the polite lead-in ("hello"): threadTitle.ts.
  await expect(page.getByTestId('threads-sidebar').getByText('Runtime', { exact: true }).first()).toBeVisible();

  // An approval: the timeline marks where the agent asked, the attention dock
  // above the composer holds the one set of answers; Approve runs the command.
  const composer = thread.getByTestId('runtime-composer');
  await composer.fill('ask');
  await composer.press('Enter');
  const approval = thread.getByTestId('runtime-attention');
  await expect(approval).toBeVisible({ timeout: 15_000 });
  await expect(approval).toContainText('Codex wants to run');
  await expect(approval).toContainText('make clean');
  await expect(thread.locator('[data-item-type="command_execution"]').getByText('Waiting for your approval')).toBeVisible();
  await expect(thread.getByRole('button', { name: 'Allow once', exact: true })).toHaveCount(1);
  await page.screenshot({ path: info.outputPath('runtime-approval-light.png'), animations: 'disabled' });
  // Dark: the secondary actions are dark buttons (once the theme transition settles).
  await page.emulateMedia({ colorScheme: 'dark' });
  const decline = approval.getByRole('button', { name: 'Deny', exact: true });
  await expect.poll(() => bgLuminance(decline)).toBeLessThan(0.35);
  await page.screenshot({ path: info.outputPath('runtime-approval-dark.png'), animations: 'disabled' });
  await page.emulateMedia({ colorScheme: 'light' });
  await approval.getByRole('button', { name: 'Allow once', exact: true }).click();
  await expect(approval).toHaveCount(0);
  const command = thread.locator('[data-item-type="command_execution"]').last();
  await expect(command).toBeVisible();
  await expect(thread.getByTestId('runtime-send')).toBeVisible({ timeout: 15_000 });

  // A turn that keeps running: Stop shows; a message sent meanwhile is queued
  // and goes out once the turn has stopped.
  await composer.fill('hang');
  await composer.press('Enter');
  await expect(thread.getByTestId('runtime-stop')).toBeVisible({ timeout: 15_000 });
  await expect(thread.getByTestId('runtime-working')).toContainText(/Working · .+ · \d+s/);
  await composer.fill('after the stop');
  await composer.press('Enter');
  await expect(thread.getByTestId('runtime-queued')).toContainText('after the stop');
  await thread.getByTestId('runtime-stop').click();
  await expect(thread.getByText('echo: after the stop')).toBeVisible({ timeout: 20_000 });
  await expect(thread.getByTestId('runtime-queued')).toHaveCount(0);

  // Slash commands: the popup lists the view's commands; /btw opens a side
  // chat about this thread without interrupting it ("Ask about <title> … Starting…"
  // while spawning, then "About <title>" once the side session is live).
  await composer.fill('/');
  await expect(thread.locator('[data-slash-pop]')).toContainText('/btw');
  await composer.fill('/btw what changed so far?');
  await composer.press('Enter');
  await expect(page.getByTestId('threads-side-chat')).toContainText(/about runtime/i, { timeout: 15_000 });
  await expect(composer).toHaveValue('');

  await expectNoHorizontalOverflow(page);
  await page.screenshot({ path: info.outputPath('runtime-thread-light.png'), animations: 'disabled' });
  await page.emulateMedia({ colorScheme: 'dark' });
  await page.screenshot({ path: info.outputPath('runtime-thread-dark.png'), animations: 'disabled' });
  await page.emulateMedia({ colorScheme: 'light' });

  // A reload replays the conversation from the daemon (snapshot / events).
  await page.reload();
  await expect(thread).toBeVisible({ timeout: 30_000 });
  await expect(thread.getByText('echo: after the stop')).toBeVisible({ timeout: 15_000 });
  await expect(thread.getByText('echo: hello runtime')).toBeVisible();

  // Model + effort for the next turn and an attached image ride the prompt.
  await thread.getByTestId('runtime-model').click();
  const mp = page.getByTestId('model-picker');
  // A17: column rows keep their 30px; they used to stretch to the provider rail
  // (Reasoning is main's segmented control now; its buttons stay as short).
  for (const row of [mp.locator('.mp-row', { hasText: 'Default model' }), mp.getByTestId('picker-effort').getByRole('button').first()]) {
    expect((await row.boundingBox())!.height).toBeLessThanOrEqual(32);
  }
  const pick = mp.locator('.mp-row:not(.cur)').filter({ hasNotText: 'Default model' }).first();
  const pickedLabel = (await pick.locator('.mp-name').innerText()).trim();
  await pick.click();
  await mp.getByTestId('picker-effort').getByRole('button', { name: 'High', exact: true }).click();
  await page.keyboard.press('Escape');
  await expect(thread.getByTestId('runtime-model')).toContainText(pickedLabel);
  const png = Buffer.from('iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNk+M9QDwADhgGAWjR9awAAAABJRU5ErkJggg==', 'base64');
  await thread.locator('input[type=file]').setInputFiles({ name: 'dot.png', mimeType: 'image/png', buffer: png });
  await expect(thread.getByTestId('runtime-images').getByRole('img', { name: 'dot.png' })).toBeVisible();
  await page.screenshot({ path: info.outputPath('runtime-composer-light.png'), animations: 'disabled' });
  await page.emulateMedia({ colorScheme: 'dark' });
  await page.screenshot({ path: info.outputPath('runtime-composer-dark.png'), animations: 'disabled' });
  await page.emulateMedia({ colorScheme: 'light' });
  const sent = page.waitForRequest((r) => r.method() === 'POST' && r.url().endsWith('/prompt'));
  await composer.fill('with a picture');
  await composer.press('Enter');
  const body = (await sent).postDataJSON() as { model?: string; effort?: string; images?: { mime: string }[] };
  expect(body.model).toBeTruthy();
  expect(body.effort).toBe('high');
  expect(body.images?.map((i) => i.mime)).toEqual(['image/png']);
  await expect(thread.getByTestId('runtime-images')).toHaveCount(0);
  await expect(thread.getByText('echo: with a picture')).toBeVisible({ timeout: 15_000 });

  // The thread's terminal drawer: a shell in the thread's folder.
  await page.getByTestId('threads-thread-menu').click();
  await page.locator('.ctx-menu').getByRole('menuitemcheckbox', { name: 'Terminal drawer' }).click();
  await expect(page.locator('[data-testid^="thread-terminal-drawer-"]')).toBeVisible({ timeout: 15_000 });
  await page.screenshot({ path: info.outputPath('runtime-terminal-light.png'), animations: 'disabled' });

  // Rename, then close: the thread moves to History; opening it from
  // History shows it read-only, and Revive brings its agent back.
  await page.getByTestId('threads-thread-menu').click();
  await page.locator('.ctx-menu').getByRole('menuitem', { name: 'Rename…' }).click();
  await page.locator('[role="dialog"] input').first().fill('Renamed runtime');
  await page.getByRole('button', { name: 'Rename', exact: true }).last().click();
  await expect(page.getByTestId('threads-breadcrumb')).toContainText('Renamed runtime');
  await page.getByTestId('threads-thread-menu').click();
  await page.locator('.ctx-menu').getByRole('menuitem', { name: 'Close thread…' }).click();
  await page.getByRole('dialog').getByRole('button', { name: 'Close thread', exact: true }).click();
  const sidebar = page.getByTestId('threads-sidebar');
  await sidebar.getByTestId('threads-history-toggle').click();
  await sidebar.locator('[data-testid^="thread-history-"]', { hasText: 'Renamed runtime' }).getByRole('button').first().click();
  await expect(thread.getByTestId('runtime-closed')).toBeVisible();
  await thread.getByTestId('runtime-reopen').click();
  await expect(thread.getByTestId('runtime-closed')).toHaveCount(0, { timeout: 15_000 });
  await expect(thread.getByTestId('runtime-composer')).toBeEnabled();
  await expect(thread.getByText('echo: hello runtime')).toBeVisible();

  // A runtime approval raises the Threads "needs you" badge while you're
  // elsewhere, and the count survives a reload before Threads is opened.
  await page.goto('/#/home');
  const created = await ctx.post(`${base}/api/v1/workspaces/${wsId}/runtime-sessions`, {
    data: { provider: 'codex', cwd: fixtureDir, title: 'Background ask' },
  });
  expect(created.ok(), await created.text()).toBeTruthy();
  const bgId = (await created.json()).id as string;
  expect((await ctx.post(`${base}/api/v1/runtime-sessions/${bgId}/prompt`, { data: { text: 'ask' } })).ok()).toBeTruthy();
  // Other specs' threads wait on the same daemon, so the count is theirs too:
  // the badge shows a count, and its source (the persisted waiting list the
  // badge counts) holds THIS thread — before and after a reload.
  const threadsNav = page.getByRole('button', { name: /^Threads ?, [1-9]\d* waiting on you/ }).first();
  const waitingIds = () => page.evaluate(() => (JSON.parse(localStorage.getItem('otto_runtime_waiting') ?? '[]') as { id: string }[]).map((w) => w.id));
  await expect(threadsNav).toBeVisible({ timeout: 15_000 });
  await expect.poll(waitingIds, { timeout: 15_000 }).toContain(bgId);
  await page.reload();
  await expect(threadsNav).toBeVisible({ timeout: 30_000 });
  await expect.poll(waitingIds, { timeout: 30_000 }).toContain(bgId);
  await page.goto('/#/threads');

  // Phone width: the timeline and composer fit.
  await page.setViewportSize({ width: 390, height: 844 });
  await expect(thread.getByTestId('runtime-composer')).toBeVisible();
  await expectNoHorizontalOverflow(page);
});

// ─────────────────────────────────────────────────────────────────────────────
// A Claude thread (otto-fake-claude, via global-setup's claude shim): work it
// starts in the background keeps it busy past its turn and the agent goes on
// by itself when that work ends; a background subagent is a card with its
// own activity; a closed thread reopens when you send to it; Delete honours
// Settings › "Closing a session tab" = Always delete.
// ─────────────────────────────────────────────────────────────────────────────
test('a new runtime thread sends the stage’s image with its first message', async ({ page }) => {
  await page.goto('/#/threads');
  await page.getByTestId('threads-new-btn').click();
  const stage = page.getByTestId('new-thread-stage');
  await expect(stage).toBeVisible({ timeout: 30_000 });
  await stage.getByTestId('picker-agent').click();
  await page.getByTestId('model-picker').getByTestId('model-picker-provider-codex').click();
  await page.keyboard.press('Escape');

  const png = Buffer.from('iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNk+M9QDwADhgGAWjR9awAAAABJRU5ErkJggg==', 'base64');
  await stage.locator('input[type=file]').setInputFiles({ name: 'first.png', mimeType: 'image/png', buffer: png });
  await expect(stage.getByRole('img', { name: 'first.png' })).toBeVisible();
  const sent = page.waitForRequest((r) => r.method() === 'POST' && r.url().endsWith('/prompt'));
  await stage.getByTestId('new-thread-input').fill('first with a picture');
  await stage.getByTestId('new-thread-send-btn').click();
  const body = (await sent).postDataJSON() as { text?: string; images?: { mime: string; name?: string }[] };
  expect(body.text).toBe('first with a picture');
  expect(body.images?.map((i) => [i.name, i.mime])).toEqual([['first.png', 'image/png']]);
  await expect(page.locator('[data-testid^="runtime-thread-"]').getByText('echo: first with a picture')).toBeVisible({ timeout: 15_000 });
});

test('a Claude thread: background work, its own turn, a subagent card, reopen and delete', async ({ page }, info) => {
  const created = await ctx.post(`${base}/api/v1/workspaces/${wsId}/runtime-sessions`, {
    data: { provider: 'claude', cwd: fixtureDir, title: 'Claude background' },
  });
  expect(created.ok(), await created.text()).toBeTruthy();
  const id = (await created.json()).id as string;
  await page.goto(`/#/threads/${id}`);
  const thread = page.getByTestId(`runtime-thread-${id}`);
  await expect(thread).toBeVisible({ timeout: 30_000 });
  const composer = thread.getByTestId('runtime-composer');

  // The turn ends with a background command still running: the thread says
  // so, then continues on its own when the command finishes.
  await composer.fill('bg 2');
  await composer.press('Enter');
  await expect(thread.getByText('Waiting for it.')).toBeVisible({ timeout: 15_000 });
  const bg = thread.getByTestId('runtime-background');
  await expect(bg).toContainText('sleep 300');
  await page.screenshot({ path: info.outputPath('runtime-background-light.png'), animations: 'disabled' });
  await expect(thread.getByText('Background done: ok')).toBeVisible({ timeout: 20_000 });
  await expect(bg).toHaveCount(0);
  await expect(thread.getByText('Background command finished: sleep 300')).toBeVisible();

  // A background subagent: a card that runs past its turn, its activity nested.
  await composer.fill('agent 2');
  await composer.press('Enter');
  const card = thread.locator('[data-testid^="runtime-subagent-"]');
  await expect(card).toContainText('PR sweep', { timeout: 15_000 });
  await expect(thread.getByText("I'm waiting for the PR sweep to come back.")).toBeVisible();
  await expect(card).toHaveAttribute('data-status', 'running');
  // A subagent like any other: "Working", never "Background".
  await expect(card).toContainText('Working');
  await expect(card).not.toContainText('Background');
  await card.getByRole('button', { name: /PR sweep/ }).click();
  await expect(card).toContainText('Looking at PRs');
  await expectNoHorizontalOverflow(page);
  await page.emulateMedia({ colorScheme: 'dark' });
  await page.screenshot({ path: info.outputPath('runtime-subagent-dark.png'), animations: 'disabled' });
  await page.emulateMedia({ colorScheme: 'light' });
  await page.screenshot({ path: info.outputPath('runtime-subagent-light.png'), animations: 'disabled' });
  await expect(card).toHaveAttribute('data-status', 'completed', { timeout: 20_000 });
  await expect(card).toContainText('Found 2 open PRs');
  await expect(thread.getByText('Agent reported')).toBeVisible({ timeout: 15_000 });

  // Close, then just send: the thread reopens and the agent answers.
  await page.getByTestId('threads-thread-menu').click();
  await page.locator('.ctx-menu').getByRole('menuitem', { name: 'Close thread…' }).click();
  await page.getByRole('dialog').getByRole('button', { name: 'Close thread', exact: true }).click();
  await expect(thread).toHaveCount(0); // its pane closes; the link still shows it
  await page.goto(`/#/threads/${id}`);
  await expect(thread.getByTestId('runtime-closed')).toBeVisible({ timeout: 15_000 });
  await expect(composer).toBeEnabled();
  await page.getByTestId('threads-thread-menu').click();
  await expect(page.locator('.ctx-menu').getByRole('menuitem', { name: 'Reopen thread' })).toBeVisible();
  await page.keyboard.press('Escape');
  // Explicit revival does not send the unfinished draft or erase prior answers.
  await composer.fill('unfinished after close');
  await thread.getByTestId('runtime-reopen').click();
  await expect(thread.getByTestId('runtime-closed')).toHaveCount(0);
  await expect(composer).toHaveValue('unfinished after close');
  await expect(thread.getByText('echo: unfinished after close')).toHaveCount(0);
  await composer.fill('after reopening');
  await composer.press('Enter');
  await expect(thread.getByText('echo: after reopening')).toBeVisible({ timeout: 20_000 });
  await expect(thread.getByTestId('runtime-closed')).toHaveCount(0);

  // Delete asks first …
  await page.getByTestId('threads-thread-menu').click();
  await page.locator('.ctx-menu').getByRole('menuitem', { name: 'Delete thread…' }).click();
  const dlg = page.getByRole('dialog');
  await expect(dlg).toContainText('Claude background');
  await dlg.getByRole('button', { name: 'Cancel' }).click();
  expect((await ctx.get(`${base}/api/v1/runtime-sessions/${id}`)).ok()).toBeTruthy();
  // … unless closing a tab is set to "Always delete".
  await page.evaluate(() => localStorage.setItem('otto_close_tab_pref', 'delete'));
  await page.reload();
  await expect(thread).toBeVisible({ timeout: 30_000 });
  await page.getByTestId('threads-thread-menu').click();
  await page.locator('.ctx-menu').getByRole('menuitem', { name: 'Delete thread…' }).click();
  await expect(page.getByRole('dialog')).toHaveCount(0);
  await expect.poll(async () => (await ctx.get(`${base}/api/v1/runtime-sessions/${id}`)).status(), { timeout: 15_000 }).toBe(404);
  await expect(thread).toHaveCount(0);
  await page.evaluate(() => localStorage.removeItem('otto_close_tab_pref'));
});

// ─────────────────────────────────────────────────────────────────────────────
// Runtime polish (mocked replay — the fake CLIs don't emit these fields yet):
// a Hebrew conversation reads right-to-left with code / chips isolated, the
// project card opens from the toolbar without covering any message and
// closes on an outside click, parallel tool calls group under one summary,
// an approval folds into its tool's row (pending: inline Approve / Decline;
// withdrawn: "Withdrawn by Claude"), a failed tool shows its error, the user
// message shows its attachments, and the access chip rides the next prompt.
// ─────────────────────────────────────────────────────────────────────────────
test('runtime polish: RTL, details card, grouped tools, folded approvals, access mode', async ({ page }, info) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  const created = await ctx.post(`${base}/api/v1/workspaces/${wsId}/runtime-sessions`, {
    data: { provider: 'codex', cwd: fixtureDir, title: 'שיחה בעברית' },
  });
  expect(created.ok(), await created.text()).toBeTruthy();
  const id = (await created.json()).id as string;

  const T0 = Date.parse('2026-11-04T10:00:00Z');
  const at = (s: number) => new Date(T0 + s * 1000).toISOString();
  const item = (o: Record<string, unknown>) => ({
    turn_id: 't1',
    status: 'completed',
    title: null,
    started_at: at(0),
    completed_at: at(1),
    updated_at: at(1),
    ...o,
  });
  const fetchTool = (n: number, o: Record<string, unknown> = {}) =>
    item({ id: `f${n}`, ordinal: 10 + n, type: 'web_search', title: 'WebFetch', patterns: [`https://example.com/${n}`], started_at: at(2), completed_at: at(6), ...o });
  const snapshot = {
    provider_session: { status: 'ready', cwd: fixtureDir, model: null, updated_at: at(0), last_error: null },
    provider_thread: null,
    turns: [{ id: 't1', ordinal: 1, status: 'completed', started_at: at(0), completed_at: at(20) }],
    turn_items: [
      item({
        id: 'u1',
        ordinal: 1,
        type: 'user_message',
        text: 'היי, מה קורה וזה? תבדוק בבקשה את הענף',
        attachments: [
          { id: 'img1', kind: 'image', mime: 'image/png', name: 'screen.png' },
          { id: 'doc1', kind: 'file', mime: 'application/pdf', name: 'spec.pdf' },
        ],
      }),
      item({
        id: 'a1',
        ordinal: 2,
        type: 'assistant_message',
        message_id: 'm1',
        streaming: false,
        text: 'אני ב-repo של Otto, על `main` בלי שינויים מקומיים. הקומיט האחרון הוא #130 (תיקון של badges).\n\nהתחלנו את המעבר של Threads ל-runtime מובנה בענף `feat/threads-structured-runtime`. מה בא לך לעשות?',
      }),
      fetchTool(1),
      fetchTool(2, { started_at: at(3), status: 'failed', error: 'HTTP 403 Forbidden: https://example.com/2' }),
      fetchTool(3, { started_at: at(4), status: 'cancelled' }),
      item({ id: 'ap3', ordinal: 14, type: 'approval_request', status: 'cancelled', cancel_reason: 'agent', tool_item_id: 'f3', request_id: 'r3', request_kind: 'permission', title: 'WebFetch', options: [] }),
      item({ id: 'c1', ordinal: 15, type: 'command_execution', input: 'git status', status: 'waiting', started_at: at(10), completed_at: null }),
      item({ id: 'ap4', ordinal: 16, type: 'approval_request', status: 'pending', tool_item_id: 'c1', request_id: 'r4', request_kind: 'command', prompt: 'git status', options: [] }),
      item({ id: 'c2', ordinal: 17, type: 'command_execution', input: 'git log -1', status: 'waiting', started_at: at(11), completed_at: null }),
      item({ id: 'ap5', ordinal: 18, type: 'approval_request', status: 'pending', tool_item_id: 'c2', request_id: 'r5', request_kind: 'command', prompt: 'git log -1', options: [] }),
    ],
    runtime_requests: [
      { id: 'r3', turn_id: 't1', kind: 'permission', status: 'cancelled', response_capability: { type: 'live' }, created_at: at(4), resolved_at: at(6), cancel_reason: 'agent' },
      { id: 'r4', turn_id: 't1', kind: 'command', status: 'pending', response_capability: { type: 'live' }, created_at: at(10), resolved_at: null },
      { id: 'r5', turn_id: 't1', kind: 'command', status: 'pending', response_capability: { type: 'live' }, created_at: at(11), resolved_at: null },
    ],
    plans: [],
  };
  await page.route(`**/api/v1/runtime-sessions/${id}/events**`, (r) =>
    r.fulfill({ json: { kind: 'snapshot', snapshot_sequence: 100_000, snapshot } }),
  );
  const png = Buffer.from('iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNk+M9QDwADhgGAWjR9awAAAABJRU5ErkJggg==', 'base64');
  await page.route(`**/api/v1/runtime-sessions/${id}/attachments/**`, (r) =>
    r.fulfill({ body: png, contentType: 'image/png' }),
  );
  const answered: string[] = [];
  await page.route(`**/api/v1/runtime-sessions/${id}/requests/**`, (r) => {
    answered.push(r.request().url().split('/').pop() ?? '');
    return r.fulfill({ status: 204 });
  });

  await page.goto('/#/threads');
  await page.getByTestId('threads-sidebar').getByText('שיחה בעברית').first().click();
  const thread = page.getByTestId(`runtime-thread-${id}`);
  await expect(thread).toBeVisible({ timeout: 30_000 });

  // A hovered sidebar row's actions sit beside the title, never over it: an
  // RTL title starts at the row's end, where the actions are revealed.
  const rowTitle = page.getByTestId('threads-sidebar').locator('.tr-title', { hasText: 'שיחה בעברית' }).first();
  await rowTitle.hover();
  const actions = rowTitle.locator('xpath=ancestor::*[contains(@class,"tr-row")][1]//*[contains(@class,"tr-actions")]');
  const [tb, ab] = [(await rowTitle.boundingBox())!, (await actions.boundingBox())!];
  const apart = tb.x + tb.width <= ab.x || ab.x + ab.width <= tb.x || tb.y + tb.height <= ab.y || ab.y + ab.height <= tb.y;
  expect(apart, 'row actions overlap the title').toBeTruthy();

  // RTL: the bubble and the answer resolve right-to-left; code / #130 stay isolated LTR.
  const bubble = thread.locator('.rt-bubble');
  const answer = thread.locator('.rt-assistant');
  await expect(bubble).toBeVisible();
  expect(await bubble.evaluate((el) => getComputedStyle(el).direction)).toBe('rtl');
  expect(await answer.locator('p').first().evaluate((el) => getComputedStyle(el).direction)).toBe('rtl');
  const code = answer.locator('code', { hasText: 'feat/threads-structured-runtime' });
  expect(await code.evaluate((el) => [getComputedStyle(el).direction, getComputedStyle(el).unicodeBidi])).toEqual(['ltr', 'isolate']);
  // The bubble hugs the inline-end (right) edge of the column.
  const col = (await thread.locator('.rt-col').boundingBox())!;
  const bb = (await bubble.boundingBox())!;
  const gutter = await thread.locator('.rt-col').evaluate(el => parseFloat(getComputedStyle(el).paddingInlineEnd));
  expect(Math.abs(col.x + col.width - (bb.x + bb.width) - gutter)).toBeLessThan(2);

  // Attachments: a lazy thumbnail and a file chip; the thumbnail opens full.
  const atts = thread.getByTestId('runtime-attachments');
  await expect(atts.locator('img')).toBeVisible();
  await expect(atts.getByText('spec.pdf')).toBeVisible();
  await atts.getByRole('button', { name: 'Open screen.png' }).click();
  await expect(page.getByRole('dialog').locator('img')).toBeVisible();
  await page.keyboard.press('Escape');
  await expect(page.getByRole('dialog')).toHaveCount(0);

  // Thread details float beside the conversation where a whole reading
  // column is left, and cover no message (LTR and RTL).
  await page.setViewportSize({ width: 1600, height: 900 });
  const card = page.getByTestId('thread-details-card');
  await expect(card).toBeVisible();
  await expectFullyInViewport(page, card, 'details card');
  const cardBox = (await card.boundingBox())!;
  for (const el of [bubble, answer, thread.getByTestId('runtime-composer')]) {
    const b = (await el.boundingBox())!;
    const overlaps = b.x < cardBox.x + cardBox.width && cardBox.x < b.x + b.width && b.y < cardBox.y + cardBox.height && cardBox.y < b.y + b.height;
    expect(overlaps, 'a message runs under the details card').toBe(false);
  }
  await page.screenshot({ path: info.outputPath('runtime-rtl-card-light.png'), animations: 'disabled' });
  await page.emulateMedia({ colorScheme: 'dark' });
  await page.screenshot({ path: info.outputPath('runtime-rtl-card-dark.png'), animations: 'disabled' });
  await page.emulateMedia({ colorScheme: 'light' });
  // A right-to-left UI mirrors the card to the inline end; it still covers nothing.
  await page.evaluate(() => localStorage.setItem('otto_direction', 'rtl'));
  await page.reload();
  await expect.poll(() => page.evaluate(() => document.documentElement.dir)).toBe('rtl');
  await expect(bubble).toBeVisible({ timeout: 30_000 });
  await expectFullyInViewport(page, card, 'details card (RTL)');
  const rtlCard = (await card.boundingBox())!;
  const rtlBubble = (await bubble.boundingBox())!;
  expect(rtlCard.x + rtlCard.width <= rtlBubble.x || rtlBubble.x + rtlBubble.width <= rtlCard.x).toBe(true);
  // An LTR label keeps its order in an RTL UI ("git log -1", not "1- git log").
  expect(await thread.getByTestId('runtime-item-c2').locator('.rw-label').evaluate((el) => getComputedStyle(el).direction)).toBe('ltr');
  await expectNoHorizontalOverflow(page);
  await page.screenshot({ path: info.outputPath('runtime-rtl-card-rtl-ui.png'), animations: 'disabled' });
  await page.evaluate(() => localStorage.removeItem('otto_direction'));
  await page.reload();
  await expect(bubble).toBeVisible({ timeout: 30_000 });
  // The user's window: a ~810 px centre column can't spare the card's width,
  // so it does not float there (it heads the right panel instead).
  await page.setViewportSize({ width: 1130, height: 900 });
  await expect(card).toHaveCount(0);
  await page.setViewportSize({ width: 1440, height: 900 });

  // Parallel fetches group under one summary; the failed one shows its error;
  // the withdrawn approval reads as such on its tool's row (no shield row).
  const group = thread.getByTestId('runtime-group-group:f1');
  await expect(group.locator('summary')).toContainText('Fetched 3 web pages');
  await group.locator('summary').click();
  await expect(thread.getByTestId('runtime-item-f3').getByTestId('runtime-approval-note')).toHaveText('Withdrawn by Codex');
  await expect(thread.locator('[data-item-type="approval_request"]')).toHaveCount(0);
  await thread.getByTestId('runtime-item-f2').locator('.rw-line').click();
  await expect(thread.getByTestId('runtime-item-f2').getByTestId('runtime-item-error')).toHaveText(/HTTP 403 Forbidden/);

  // Two pending approvals at once (parallel commands: their group opens by
  // itself): each command row is marked, the dock steps "1 of 2", and a
  // row's Answer brings that request up in the dock.
  await expect(thread.getByTestId('runtime-group-group:c1')).toHaveAttribute('open', '');
  for (const [tool, req] of [
    ['c1', 'r4'],
    ['c2', 'r5'],
  ]) {
    const row = thread.getByTestId(`runtime-item-${tool}`);
    await expect(row.getByTestId(`runtime-approval-${req}`)).toBeVisible();
  }
  const dock = thread.getByTestId('runtime-attention');
  await expect(dock.getByTestId('runtime-attention-count')).toHaveText('1 of 2');
  await expect(dock).toContainText('git status');
  await thread.getByTestId('runtime-item-c2').getByTestId('runtime-approval-answer').click();
  await expect(dock.getByTestId('runtime-attention-count')).toHaveText('2 of 2');
  await expect(dock).toContainText('git log -1');
  await dock.getByRole('button', { name: 'Allow once', exact: true }).click();
  await thread.getByTestId('runtime-item-c1').getByTestId('runtime-approval-answer').click();
  await expect(dock).toContainText('git status');
  await dock.getByRole('button', { name: 'Deny', exact: true }).click();
  await expect.poll(() => answered.sort()).toEqual(['r4', 'r5']);
  await page.screenshot({ path: info.outputPath('runtime-polish-work-light.png'), animations: 'disabled' });
  await page.emulateMedia({ colorScheme: 'dark' });
  await page.screenshot({ path: info.outputPath('runtime-polish-work-dark.png'), animations: 'disabled' });
  await page.emulateMedia({ colorScheme: 'light' });

  // (The mocked answers leave the requests pending: type first, so the
  // composer unfolds from its one-row waiting state.)
  await thread.getByTestId('runtime-composer').fill('שלום');
  // Access: Full access asks once and rides the next prompt. The chip hides
  // only for a calm default (Supervised / Auto-accept edits); Full access
  // always shows it, in amber; "+" always offers the choice.
  const chip = thread.getByTestId('runtime-access');
  if ((await chip.count()) === 0) {
    await thread.getByTestId('runtime-plus').click();
    await page.locator('.ctx-menu').getByRole('menuitemcheckbox', { name: 'Access: Supervised' }).click();
  } else if (!(await chip.textContent())?.includes('Supervised')) {
    await chip.click();
    await page.locator('.ctx-menu').getByRole('menuitemcheckbox', { name: 'Supervised' }).click();
  }
  await expect(chip).toContainText('Supervised');
  await chip.click();
  await page.locator('.ctx-menu').getByRole('menuitemcheckbox', { name: 'Full access' }).click();
  const dlg = page.getByRole('dialog');
  await expect(dlg).toContainText('without asking');
  await dlg.getByRole('button', { name: 'Allow full access' }).click();
  await expect(chip).toContainText('Full access');
  const sent = page.waitForRequest((r) => r.method() === 'POST' && r.url().endsWith(`/runtime-sessions/${id}/prompt`));
  await page.route(`**/api/v1/runtime-sessions/${id}/prompt`, (r) => r.fulfill({ json: { turn_id: 't2', sequence: 1 } }));
  const composer = thread.getByTestId('runtime-composer');
  await composer.fill('שלום');
  await composer.press('Enter');
  expect(((await sent).postDataJSON() as { access_mode?: string }).access_mode).toBe('full-access');
  // The next switch back needs no confirm.
  await thread.getByTestId('runtime-plus').click();
  await page.locator('.ctx-menu').getByRole('menuitemcheckbox', { name: 'Access: Supervised' }).click();
  await expect(chip).toContainText('Supervised');
  await expectNoHorizontalOverflow(page);
});

test('composer drafts recover after reload and page reopen without sending; failed send keeps them', async ({ page }, info) => {
  const created = await ctx.post(`${base}/api/v1/workspaces/${wsId}/runtime-sessions`, {
    data: { provider: 'codex', cwd: fixtureDir, title: 'Recover my unsent draft' },
  });
  expect(created.ok(), await created.text()).toBeTruthy();
  const id = (await created.json()).id as string;
  const png = 'iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+jWZkAAAAASUVORK5CYII=';
  let promptRequests = 0;
  page.context().on('request', r => { if (r.method() === 'POST' && r.url().endsWith(`/runtime-sessions/${id}/prompt`)) promptRequests++; });
  await page.goto('/#/threads');
  await page.getByTestId('threads-sidebar').getByText('Recover my unsent draft', { exact: true }).click();
  const thread = page.getByTestId(`runtime-thread-${id}`);
  // The save state shows only on failure; its data-state says the draft finished loading.
  await expect(thread.getByTestId('runtime-draft-status')).toHaveAttribute('data-state', 'saved');
  await thread.getByTestId('runtime-composer').fill('Unsent browser recovery text');
  await thread.locator('input[type=file]').setInputFiles({ name: 'recover.png', mimeType: 'image/png', buffer: Buffer.from(png, 'base64') });
  await expect.poll(async () => {
    const record = await (await ctx.get(`${base}/api/v1/runtime-sessions/${id}/draft`)).json();
    return [record.draft?.text, record.draft?.images?.[0]?.name];
  }).toEqual(['Unsent browser recovery text', 'recover.png']);
  await page.reload();
  await expect(thread.getByTestId('runtime-composer')).toHaveValue('Unsent browser recovery text');
  await expect(thread.getByTestId('runtime-images').getByRole('img', { name: 'recover.png' })).toBeVisible();
  expect(promptRequests).toBe(0);
  const storedValues = await page.evaluate(() => Object.values(localStorage).join('\n'));
  expect(storedValues).not.toContain('Unsent browser recovery text');
  expect(storedValues).not.toContain(png);
  const reopened = await page.context().newPage();
  await page.close();
  await reopened.goto('/#/threads');
  await reopened.getByTestId('threads-sidebar').getByText('Recover my unsent draft', { exact: true }).click();
  const recovered = reopened.getByTestId(`runtime-thread-${id}`);
  await expect(recovered.getByTestId('runtime-composer')).toHaveValue('Unsent browser recovery text');
  await expect(recovered.getByTestId('runtime-images').getByRole('img', { name: 'recover.png' })).toBeVisible();
  expect(promptRequests).toBe(0);
  await reopened.route(`**/api/v1/runtime-sessions/${id}/prompt`, route => route.fulfill({ status: 400, json: { code: 'invalid', message: 'Test provider cannot start' } }));
  await recovered.getByTestId('runtime-send').click();
  await expect(recovered).toContainText('Test provider cannot start');
  await expect(recovered.getByTestId('runtime-composer')).toHaveValue('Unsent browser recovery text');
  await expect(recovered.getByTestId('runtime-images').getByRole('img', { name: 'recover.png' })).toBeVisible();
  await reopened.screenshot({ path: info.outputPath('draft-recovered-light.png') });
  await reopened.emulateMedia({ colorScheme: 'dark' });
  await reopened.screenshot({ path: info.outputPath('draft-recovered-dark.png') });
  await reopened.close();
});

test('native Codex question forms answer near the composer on phone and desktop', async ({ page }, info) => {
  const created = await ctx.post(`${base}/api/v1/workspaces/${wsId}/runtime-sessions`, {
    data: { provider: 'codex', cwd: fixtureDir, title: 'Question forms' },
  });
  expect(created.ok(), await created.text()).toBeTruthy();
  const { id } = await created.json() as { id: string };
  await page.goto(`/#/threads/${id}`);
  const thread = page.getByTestId(`runtime-thread-${id}`);
  await expect(thread).toBeVisible({ timeout: 30_000 });
  const composer = thread.getByTestId('runtime-composer');
  await composer.fill('question form');
  await composer.press('Enter');
  const card = thread.getByTestId('runtime-question-card');
  await expect(card).toBeVisible({ timeout: 15_000 });
  await expect(thread.locator('[data-testid^="runtime-approval-"]')).toHaveCount(0);
  await card.getByTestId('runtime-question-send').click();
  await expect(card.getByRole('alert')).toContainText('Answer every required question');
  await card.getByLabel('Careful', { exact: false }).check();
  await card.getByLabel('Your answer', { exact: true }).fill('Keep the API stable');
  await page.screenshot({ path: info.outputPath('runtime-questions-light.png'), animations: 'disabled' });
  await page.emulateMedia({ colorScheme: 'dark' });
  await page.screenshot({ path: info.outputPath('runtime-questions-dark.png'), animations: 'disabled' });
  await page.setViewportSize({ width: 390, height: 844 });
  await expectNoHorizontalOverflow(page);
  // The form lives once, in the attention dock, with its Send in view.
  await expect(thread.getByTestId('runtime-attention')).toHaveAttribute('data-kind', 'question');
  await expect(thread.getByTestId('runtime-question-send')).toHaveCount(1);
  await expectFullyInViewport(page, card.getByTestId('runtime-question-send'), 'Send answer');
  await page.screenshot({ path: info.outputPath('runtime-questions-phone.png'), animations: 'disabled' });
  // A failed HTTP answer must keep form input, never fall through to ordinary chat.
  let fail = true;
  await page.route(`**/api/v1/runtime-sessions/${id}/requests/**`, async (route) => {
    if (fail) { fail = false; await route.fulfill({ status: 409, json: { error: 'Temporarily unavailable' } }); }
    else await route.continue();
  });
  await card.getByTestId('runtime-question-send').click();
  await expect(card.getByRole('alert')).toContainText('Couldn’t send the answer');
  await expect(card.getByLabel('Your answer', { exact: true })).toHaveValue('Keep the API stable');
  const sent = page.waitForRequest((r) => r.method() === 'POST' && r.url().includes(`/runtime-sessions/${id}/requests/`));
  await card.getByTestId('runtime-question-send').click();
  expect((await sent).postDataJSON()).toEqual({ answers: [
    { question_id: 'strategy_native', type: 'single', value: 'Careful' },
    { question_id: 'note_native', type: 'text', text: 'Keep the API stable' },
  ] });
  await expect(thread.getByText(/question answers:.*strategy_native/)).toBeVisible({ timeout: 15_000 });
  await expect(thread.getByTestId('runtime-question-send')).toHaveCount(0);
  await expect(composer).toHaveValue('');
});

test('threads composer typing and timeline scrolling stay lag-free with clamped long reasoning and layout isolation', async ({ page }) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  const created = await ctx.post(`${base}/api/v1/workspaces/${wsId}/runtime-sessions`, {
    data: { provider: 'codex', cwd: fixtureDir, title: 'Smooth typing & scroll' },
  });
  expect(created.ok(), await created.text()).toBeTruthy();
  const { id } = (await created.json()) as { id: string };

  const T0 = Date.parse('2026-11-14T10:00:00Z');
  const at = (s: number) => new Date(T0 + s * 1000).toISOString();
  const longReason = Array.from(
    { length: 80 },
    (_, i) => `Step ${i + 1}: analyzing crate graph, checking borrow lifetimes, and verifying SQLite WAL concurrency invariants.`,
  ).join('\n') + '\n\n```rust\npub fn tail_marker_final() -> bool { true }\n```';

  const turns = Array.from({ length: 20 }, (_, idx) => ({
    id: `t-${idx + 1}`,
    ordinal: idx + 1,
    status: 'completed',
    started_at: at(idx * 30),
    completed_at: at(idx * 30 + 20),
    origin: 'user',
  }));
  const turn_items: Record<string, unknown>[] = [];
  let ord = 1;
  for (let idx = 0; idx < 20; idx++) {
    turn_items.push({
      id: `u-${idx + 1}`,
      turn_id: `t-${idx + 1}`,
      ordinal: ord++,
      status: 'completed',
      title: null,
      started_at: at(idx * 30),
      completed_at: at(idx * 30 + 1),
      updated_at: at(idx * 30 + 1),
      type: 'user_message',
      text: `Prompt #${idx + 1}: inspect subsystem ${idx + 1}`,
    });
    if (idx === 19) {
      turn_items.push({
        id: 'r-long',
        turn_id: 't-20',
        ordinal: ord++,
        status: 'completed',
        title: 'Thinking',
        started_at: at(idx * 30 + 2),
        completed_at: at(idx * 30 + 10),
        updated_at: at(idx * 30 + 10),
        type: 'reasoning',
        text: longReason,
      });
    }
    turn_items.push({
      id: `a-${idx + 1}`,
      turn_id: `t-${idx + 1}`,
      ordinal: ord++,
      status: 'completed',
      title: null,
      started_at: at(idx * 30 + 11),
      completed_at: at(idx * 30 + 20),
      updated_at: at(idx * 30 + 20),
      type: 'assistant_message',
      message_id: `m-${idx + 1}`,
      streaming: false,
      text: `### Reply ${idx + 1}\n\nVerified module \`crate_${idx + 1}\` with zero warnings and clean allocations.`,
    });
  }

  let eventReplays = 0;
  await page.route(`**/api/v1/runtime-sessions/${id}/events**`, (route) => {
    eventReplays++;
    return route.fulfill({
      json: {
        kind: 'snapshot',
        snapshot_sequence: 500,
        snapshot: {
          provider_session: { status: 'ready', cwd: fixtureDir, model: null, updated_at: at(700), last_error: null },
          provider_thread: null,
          turns,
          turn_items,
          runtime_requests: [],
          plans: [],
        },
      },
    });
  });

  await page.goto(`/#/threads/${id}`);
  const thread = page.getByTestId(`runtime-thread-${id}`);
  await expect(thread).toBeVisible({ timeout: 30_000 });
  await expect(thread.getByText('Reply 20')).toBeVisible();

  // 1) Scroller uses CSS layout/paint containment (`contain: content` === `layout style paint`) while no modal overlay is open.
  const containCss = await thread.locator('.rt-scroll').evaluate((el) => getComputedStyle(el).contain);
  expect(
    containCss === 'content' || (containCss.includes('layout') && containCss.includes('paint')),
    `expected CSS containment on .rt-scroll, got "${containCss}"`,
  ).toBe(true);

  // 2) Long reasoning trace renders a compact preview while collapsed (no 80-line DOM bloat), and full text on "More".
  const thinkRow = thread.getByTestId('runtime-item-r-long');
  await expect(thinkRow).toBeVisible();
  const collapsedLen = await thinkRow.locator('.rw-think').innerText();
  expect(collapsedLen.length).toBeLessThan(700);
  expect(collapsedLen).not.toContain('tail_marker_final');
  await thinkRow.getByRole('button', { name: 'More' }).click();
  await expect(thinkRow.locator('.rw-think')).toContainText('tail_marker_final');
  await thinkRow.getByRole('button', { name: 'Less' }).click();

  // 3) Composer disables spellcheck/autocorrect and handles 40 rapid keystrokes with p95 < 8ms and 0 mid-typing /events GETs.
  const composer = thread.getByTestId('runtime-composer');
  expect(await composer.getAttribute('spellcheck')).toBe('false');
  const replaysBeforeTyping = eventReplays;
  const p95Ms = await composer.evaluate(async (ta: HTMLTextAreaElement) => {
    ta.focus();
    const samples: number[] = [];
    const sampleText = 'Checking runtimeView & timelineRows perf!';
    for (const ch of sampleText) {
      const t0 = performance.now();
      ta.setRangeText(ch, ta.selectionStart ?? ta.value.length, ta.selectionEnd ?? ta.value.length, 'end');
      ta.dispatchEvent(new InputEvent('input', { bubbles: true, data: ch, inputType: 'insertText' }));
      ta.dispatchEvent(new KeyboardEvent('keyup', { bubbles: true, key: ch }));
      samples.push(performance.now() - t0);
    }
    samples.sort((a, b) => a - b);
    return samples[Math.floor(samples.length * 0.95)] ?? 0;
  });
  expect(p95Ms, `composer input p95 ${p95Ms.toFixed(2)}ms exceeded 8ms`).toBeLessThan(8);
  expect(eventReplays).toBe(replaysBeforeTyping);

  // 4) Smooth programmatic timeline scroll sweep across the 20 turns.
  const scrollOk = await thread.locator('.rt-scroll').evaluate(async (el: HTMLElement) => {
    el.scrollTop = 0;
    const steps = 12;
    const max = Math.max(1, el.scrollHeight - el.clientHeight);
    for (let i = 1; i <= steps; i++) {
      el.scrollTop = Math.round((max * i) / steps);
      el.dispatchEvent(new Event('scroll'));
      await new Promise<void>((r) => requestAnimationFrame(() => r()));
    }
    return el.scrollTop > 0;
  });
  expect(scrollOk).toBe(true);
});

test('a 4,000-token stream follows the bottom with at most one layout per frame', async ({ page }) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  const created = await ctx.post(`${base}/api/v1/workspaces/${wsId}/runtime-sessions`, { data: { provider: 'codex', cwd: fixtureDir, title: 'Layout per frame' } });
  expect(created.ok(), await created.text()).toBeTruthy();
  const { id } = (await created.json()) as { id: string };
  await page.goto(`/#/threads/${id}`);
  const thread = page.getByTestId(`runtime-thread-${id}`);
  await expect(thread.getByTestId('runtime-composer')).toBeVisible({ timeout: 30_000 });
  const cdp = await page.context().newCDPSession(page);
  await cdp.send('Performance.enable');
  const layouts = async (): Promise<number> => (await cdp.send('Performance.getMetrics')).metrics.find((m) => m.name === 'LayoutCount')!.value;
  await page.evaluate(() => {
    const w = window as unknown as { __frames: number; __counting: boolean };
    w.__frames = 0;
    w.__counting = true;
    const tick = (): void => { if (!w.__counting) return; w.__frames++; requestAnimationFrame(tick); };
    requestAnimationFrame(tick);
  });
  const l0 = await layouts();
  const sent = await ctx.post(`${base}/api/v1/runtime-sessions/${id}/prompt`, { data: { text: 'stream:4000:3' } });
  expect(sent.ok(), await sent.text()).toBeTruthy();
  await expect(thread.getByText('c3999', { exact: false }).last()).toBeVisible({ timeout: 120_000 });
  const forced = (await layouts()) - l0;
  const frames = await page.evaluate(() => { const w = window as unknown as { __frames: number; __counting: boolean }; w.__counting = false; return w.__frames; });
  // A layout per frame is the browser's own; more means something read
  // layout between events (the follow used to, once per streamed event).
  expect(forced, `${forced} layouts over ${frames} frames`).toBeLessThanOrEqual(frames + 10);
  // Still following: the end of the answer is in view.
  const scroller = thread.locator('.rt-scroll');
  await expect.poll(() => scroller.evaluate((el) => el.scrollHeight - el.scrollTop - el.clientHeight)).toBeLessThan(40);
});
