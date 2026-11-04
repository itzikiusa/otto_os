import { test, expect, type APIRequestContext } from '@playwright/test';
import { mkdtempSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { apiCtx } from './seed';
import { expectNoHorizontalOverflow } from './helpers';
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
  const body = (await sent).postDataJSON() as { model?: string; effort?: string; images?: { media_type: string }[] };
  expect(body.model).toBeTruthy();
  expect(body.effort).toBe('high');
  expect(body.images?.map((i) => i.media_type)).toEqual(['image/png']);
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
