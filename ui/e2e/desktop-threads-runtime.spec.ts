import { test, expect, type APIRequestContext } from '@playwright/test';
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
  await expect(page.getByTestId('threads-sidebar').getByText('hello runtime').first()).toBeVisible();

  // An approval shows inline where the agent asked; Approve runs the command.
  const composer = thread.getByTestId('runtime-composer');
  await composer.fill('ask');
  await composer.press('Enter');
  const approval = thread.locator('[data-testid^="runtime-approval-"]');
  await expect(approval).toBeVisible({ timeout: 15_000 });
  await expect(approval.getByText('Waiting for approval')).toBeVisible();
  await page.screenshot({ path: info.outputPath('runtime-approval-light.png'), animations: 'disabled' });
  // Dark: the secondary actions are dark buttons (once the theme transition settles).
  await page.emulateMedia({ colorScheme: 'dark' });
  const decline = approval.getByRole('button', { name: 'Decline', exact: true });
  await expect.poll(() => bgLuminance(decline)).toBeLessThan(0.35);
  await page.screenshot({ path: info.outputPath('runtime-approval-dark.png'), animations: 'disabled' });
  await page.emulateMedia({ colorScheme: 'light' });
  await approval.getByRole('button', { name: 'Approve', exact: true }).click();
  await expect(approval).toHaveCount(0);
  const command = thread.locator('[data-item-type="command_execution"]').last();
  await expect(command).toBeVisible();
  await expect(thread.getByTestId('runtime-send')).toBeVisible({ timeout: 15_000 });

  // A turn that keeps running: Stop shows; a message sent meanwhile is queued
  // and goes out once the turn has stopped.
  await composer.fill('hang');
  await composer.press('Enter');
  await expect(thread.getByTestId('runtime-stop')).toBeVisible({ timeout: 15_000 });
  await expect(thread.getByText(/Working for \d+s/)).toBeVisible();
  await composer.fill('after the stop');
  await composer.press('Enter');
  await expect(thread.getByTestId('runtime-queued')).toContainText('after the stop');
  await thread.getByTestId('runtime-stop').click();
  await expect(thread.getByText('echo: after the stop')).toBeVisible({ timeout: 20_000 });
  await expect(thread.getByTestId('runtime-queued')).toHaveCount(0);

  // Slash commands: the popup lists the view's commands; /btw opens a side
  // chat about this thread without interrupting it.
  await composer.fill('/');
  await expect(thread.locator('[data-slash-pop]')).toContainText('/btw');
  await composer.fill('/btw what changed so far?');
  await composer.press('Enter');
  // (The side chat itself is a PTY thread — its agent never starts against
  // the e2e stand-in CLI, so this checks that it is asked about this thread.)
  await expect(page.getByTestId('threads-side-chat')).toContainText('Ask about hello runtime', { timeout: 15_000 });
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
  const pick = mp.locator('.mp-row:not(.cur)').filter({ hasNotText: 'Default model' }).first();
  const pickedLabel = (await pick.locator('.mp-name').innerText()).trim();
  await pick.click();
  await mp.getByTestId('picker-effort').getByRole('button', { name: 'High', exact: true }).click();
  await page.keyboard.press('Escape');
  await expect(thread.getByTestId('runtime-model')).toContainText(pickedLabel);
  const png = Buffer.from('iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNk+M9QDwADhgGAWjR9awAAAABJRU5ErkJggg==', 'base64');
  await thread.locator('input[type=file]').setInputFiles({ name: 'dot.png', mimeType: 'image/png', buffer: png });
  await expect(thread.getByTestId('runtime-images')).toContainText('dot.png');
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
  await page.locator('.ctx-menu').getByRole('menuitem', { name: 'Open terminal drawer' }).click();
  await expect(page.locator('[data-testid^="thread-terminal-drawer-"]')).toBeVisible({ timeout: 15_000 });
  await page.screenshot({ path: info.outputPath('runtime-terminal-light.png'), animations: 'disabled' });

  // Rename, then close: the thread moves to History and opens read-only.
  await page.getByTestId('threads-thread-menu').click();
  await page.locator('.ctx-menu').getByRole('menuitem', { name: 'Rename…' }).click();
  await page.locator('[role="dialog"] input').first().fill('Renamed runtime');
  await page.getByRole('button', { name: 'Rename', exact: true }).last().click();
  await expect(page.getByTestId('threads-breadcrumb')).toContainText('Renamed runtime');
  await page.getByTestId('threads-thread-menu').click();
  await page.locator('.ctx-menu').getByRole('menuitem', { name: 'Close thread…' }).click();
  await page.getByRole('dialog').getByRole('button', { name: 'Close thread', exact: true }).click();
  const sidebar = page.getByTestId('threads-sidebar');
  await sidebar.locator('.th-history').click();
  await sidebar.locator('[data-testid^="thread-history-"]', { hasText: 'Renamed runtime' }).getByRole('button').first().click();
  await expect(thread.getByTestId('runtime-composer')).toBeDisabled({ timeout: 15_000 });
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
  const threadsNav = page.getByRole('button', { name: /^Threads ?, 1 waiting on you/ }).first();
  await expect(threadsNav).toBeVisible({ timeout: 15_000 });
  await page.reload();
  await expect(threadsNav).toBeVisible({ timeout: 30_000 });
  await page.goto('/#/threads');

  // Phone width: the timeline and composer fit.
  await page.setViewportSize({ width: 390, height: 844 });
  await expect(thread.getByTestId('runtime-composer')).toBeVisible();
  await expectNoHorizontalOverflow(page);
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
test('runtime polish: RTL, project card, grouped tools, folded approvals, access mode', async ({ page }, info) => {
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
  expect(col.x + col.width - (bb.x + bb.width)).toBeLessThan(24);

  // Attachments: a lazy thumbnail and a file chip; the thumbnail opens full.
  const atts = thread.getByTestId('runtime-attachments');
  await expect(atts.locator('img')).toBeVisible();
  await expect(atts.getByText('spec.pdf')).toBeVisible();
  await atts.getByRole('button', { name: 'Open screen.png' }).click();
  await expect(page.getByRole('dialog').locator('img')).toBeVisible();
  await page.keyboard.press('Escape');
  await expect(page.getByRole('dialog')).toHaveCount(0);

  // The project card opens from the toolbar and covers no message.
  await page.getByTestId('threads-toggle-card').click();
  const card = page.getByTestId('threads-project-card');
  await expectFullyInViewport(page, card, 'project card');
  const cardBox = (await card.boundingBox())!;
  for (const el of [bubble, answer, thread.getByTestId('runtime-composer')]) {
    const b = (await el.boundingBox())!;
    const overlaps = b.x < cardBox.x + cardBox.width && cardBox.x < b.x + b.width && b.y < cardBox.y + cardBox.height && cardBox.y < b.y + b.height;
    expect(overlaps, 'a message runs under the project card').toBe(false);
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
  await page.getByTestId('threads-toggle-card').click();
  await expectFullyInViewport(page, card, 'project card (RTL)');
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
  await page.getByTestId('threads-toggle-card').click();
  await expect(card).toBeVisible();
  // Outside click closes it (and Esc after reopening).
  await thread.locator('.rt-col').click({ position: { x: 10, y: 10 } });
  await expect(card).toHaveCount(0);
  await page.getByTestId('threads-toggle-card').click();
  await expect(card).toBeVisible();
  await page.keyboard.press('Escape');
  await expect(card).toHaveCount(0);

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
  // itself), each inline on its command and each answerable.
  await expect(thread.getByTestId('runtime-group-group:c1')).toHaveAttribute('open', '');
  for (const [tool, req] of [
    ['c1', 'r4'],
    ['c2', 'r5'],
  ]) {
    const row = thread.getByTestId(`runtime-item-${tool}`);
    await expect(row.getByTestId(`runtime-approval-${req}`)).toBeVisible();
  }
  await thread.getByTestId('runtime-item-c2').getByRole('button', { name: 'Approve', exact: true }).click();
  await thread.getByTestId('runtime-item-c1').getByRole('button', { name: 'Decline', exact: true }).click();
  await expect.poll(() => answered.sort()).toEqual(['r4', 'r5']);
  await page.screenshot({ path: info.outputPath('runtime-polish-work-light.png'), animations: 'disabled' });
  await page.emulateMedia({ colorScheme: 'dark' });
  await page.screenshot({ path: info.outputPath('runtime-polish-work-dark.png'), animations: 'disabled' });
  await page.emulateMedia({ colorScheme: 'light' });

  // Access chip: Full access asks once, warns, and rides the next prompt.
  const chip = thread.getByTestId('runtime-access');
  await expect(chip).toContainText('Supervised');
  await chip.click();
  await page.locator('.ctx-menu').getByRole('menuitemcheckbox', { name: 'Full access' }).click();
  const dlg = page.getByRole('dialog');
  await expect(dlg).toContainText('without asking');
  await dlg.getByRole('button', { name: 'Allow full access' }).click();
  await expect(chip).toContainText('Full access');
  await expect(chip).toHaveClass(/full/);
  const sent = page.waitForRequest((r) => r.method() === 'POST' && r.url().endsWith(`/runtime-sessions/${id}/prompt`));
  await page.route(`**/api/v1/runtime-sessions/${id}/prompt`, (r) => r.fulfill({ json: { turn_id: 't2', sequence: 1 } }));
  const composer = thread.getByTestId('runtime-composer');
  await composer.fill('שלום');
  await composer.press('Enter');
  expect(((await sent).postDataJSON() as { access_mode?: string }).access_mode).toBe('full-access');
  // The next switch back needs no confirm.
  await chip.click();
  await page.locator('.ctx-menu').getByRole('menuitemcheckbox', { name: 'Supervised' }).click();
  await expect(chip).toContainText('Supervised');
  await expectNoHorizontalOverflow(page);
});
