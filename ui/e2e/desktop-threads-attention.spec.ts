import { test, expect, type APIRequestContext, type Page } from '@playwright/test';
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { apiCtx } from './seed';
import { expectFullyInViewport, expectNoHorizontalOverflow } from './helpers';

// ─────────────────────────────────────────────────────────────────────────────
// One structured thread as a person meets it, with REAL fake-agent turns
// (otto-fake-codex: `ask` = a command approval, `hang` = a turn to Stop,
// `question form` = a two-part question, `crash` = the agent dies mid-turn):
//   • "the agent is waiting for you" docks above the composer — one line of
//     who/what/when and the answers right there; ⌘↵ answers from an empty
//     composer; the timeline keeps a marker, never a second set of buttons;
//   • Stop stays reachable while text is typed (and on top), Esc-Esc stops;
//   • a question form's Send is inside the dock and in view, exactly once;
//   • a failed turn is ONE calm block (cause in words, raw text behind
//     Details, no "0ms", no second "Agent stopped" line).
// Screenshots (OTTO_E2E_SHOTS/fix-T-THREAD/) at 1280×800, light and dark.
// ─────────────────────────────────────────────────────────────────────────────

let ctx: APIRequestContext;
let base = '';
let wsId = '';
let fixtureDir = '';

test.beforeAll(async () => {
  const c = await apiCtx();
  ctx = c.ctx;
  base = c.base;
  fixtureDir = mkdtempSync(join(tmpdir(), 'otto-e2e-threads-attn-'));
  writeFileSync(join(fixtureDir, 'README.md'), '# Attention\n');
  const r = await ctx.post(`${base}/api/v1/workspaces`, { data: { name: 'Attention WS', root_path: fixtureDir } });
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
  await page.setViewportSize({ width: 1280, height: 800 });
  await page.addInitScript((id) => {
    if (sessionStorage.getItem('threads-attn-init')) return;
    sessionStorage.setItem('threads-attn-init', '1');
    localStorage.setItem('otto_workspace', id as string);
    localStorage.removeItem('otto_threads_v1');
  }, wsId);
});

async function shot(page: Page, name: string): Promise<void> {
  const dir = process.env.OTTO_E2E_SHOTS;
  if (!dir) return;
  mkdirSync(join(dir, 'fix-T-THREAD'), { recursive: true });
  for (const scheme of ['light', 'dark'] as const) {
    await page.emulateMedia({ colorScheme: scheme });
    await page.screenshot({ path: join(dir, 'fix-T-THREAD', `desktop-${name}-${scheme}.png`), animations: 'disabled' });
  }
  await page.emulateMedia({ colorScheme: 'light' });
}

async function openThread(page: Page, title: string) {
  const created = await ctx.post(`${base}/api/v1/workspaces/${wsId}/runtime-sessions`, {
    data: { provider: 'codex', cwd: fixtureDir, title },
  });
  expect(created.ok(), await created.text()).toBeTruthy();
  const id = (await created.json()).id as string;
  await page.goto(`/#/threads/${id}`);
  const thread = page.getByTestId(`runtime-thread-${id}`);
  await expect(thread).toBeVisible({ timeout: 30_000 });
  return { id, thread, composer: thread.getByTestId('runtime-composer') };
}

/** The element at a locator's centre is the locator itself (or inside it). */
async function onTop(page: Page, loc: import('@playwright/test').Locator): Promise<boolean> {
  const box = (await loc.boundingBox())!;
  return loc.evaluate((el, p) => {
    const hit = document.elementFromPoint(p.x, p.y);
    return !!hit && (hit === el || el.contains(hit));
  }, { x: box.x + box.width / 2, y: box.y + box.height / 2 });
}

test('an approval docks above the composer and ⌘↵ answers it from an empty composer', async ({ page }) => {
  const { thread, composer } = await openThread(page, 'Approval dock');
  await composer.fill('ask');
  await composer.press('Enter');
  const dock = thread.getByTestId('runtime-attention');
  await expect(dock).toBeVisible({ timeout: 15_000 });
  await expect(dock).toHaveAttribute('data-kind', 'approval');
  await expect(dock).toContainText('Codex wants to run');
  await expect(dock).toContainText('make clean');
  await expect(dock).toContainText('clean the build');
  // Directly above the composer, inside the thread (not floating over the timeline).
  const [d, c, s] = [(await dock.boundingBox())!, (await thread.locator('.rt-composer').boundingBox())!, (await thread.locator('.rt-scroll').boundingBox())!];
  expect(d.y + d.height).toBeLessThanOrEqual(c.y + 1);
  expect(d.y).toBeGreaterThanOrEqual(s.y + s.height - 1);
  // One set of answers on screen; the timeline row is the record.
  await expect(thread.getByRole('button', { name: 'Approve', exact: true })).toHaveCount(1);
  await expect(thread.locator('[data-item-type="command_execution"]').getByText('Waiting for your approval')).toBeVisible();
  // Stop steps back: the dock's Approve is the one primary action.
  await expect(thread.getByTestId('runtime-stop')).toHaveClass(/quiet/);
  await expect(composer).toHaveAttribute('placeholder', 'Answer above, or type to send a message instead');
  // One place: the timeline keeps a quiet line, no second box; the composer
  // folds to one row and focus is on the dock's primary action.
  await expect(thread.locator('.ra:not(.ra-docked)')).toHaveCount(0);
  await expect(thread.getByTestId('runtime-approval-answer')).toContainText('Waiting for your approval');
  await expect(composer).toHaveAttribute('rows', '1');
  await expect(dock.getByTestId('runtime-dock-accept')).toBeFocused();
  // Desktop hints on the answers.
  await expect(dock.getByTestId('runtime-dock-accept')).toContainText(/⌘↵|Ctrl\+↵/);
  await expect(dock.getByRole('button', { name: 'Approve', exact: true })).toBeVisible();
  expect(await onTop(page, dock.getByRole('button', { name: 'Approve', exact: true }))).toBe(true);
  await shot(page, 'approval-dock');

  // Typing in the composer never answers it…
  await composer.fill('a thought');
  await composer.press('Enter'); // queues (the default mid-turn delivery; ⌘↵ would steer)
  await expect(dock).toBeVisible();
  await expect(thread.getByTestId('runtime-queued')).toContainText('a thought');
  // … an empty composer's ⌘↵ does.
  await composer.fill('');
  await composer.focus();
  await composer.press('ControlOrMeta+Enter');
  await expect(dock).toHaveCount(0, { timeout: 15_000 });
  await expect(thread.locator('[data-item-type="command_execution"]').last()).toBeVisible();
  await expect(thread.getByText('echo: a thought')).toBeVisible({ timeout: 20_000 });
  await expectNoHorizontalOverflow(page);
});

test('while a turn runs, Stop stays on top next to Send and Esc-Esc stops it', async ({ page }) => {
  const { thread, composer } = await openThread(page, 'Stop is reachable');
  await composer.fill('hang');
  await composer.press('Enter');
  const stop = thread.getByTestId('runtime-stop');
  await expect(stop).toBeVisible({ timeout: 15_000 });
  await expect(thread.getByTestId('runtime-working')).toBeVisible();
  await composer.fill('next idea');
  // Both: Send queues, Stop stops — Stop never disappears behind the text.
  await expect(stop).toBeVisible();
  await expect(thread.getByTestId('runtime-send')).toBeVisible();
  expect(await onTop(page, stop)).toBe(true);
  // The composer has a visible focus ring, not just a border shade.
  await composer.focus();
  expect(await thread.locator('.rt-composer').evaluate((el) => getComputedStyle(el).boxShadow)).not.toBe('none');
  await shot(page, 'running-typed');
  // One Esc with text keeps the turn; a second within the window stops it.
  await composer.press('Escape');
  await page.waitForTimeout(700); // ui-guards: allow — proves one Esc did NOT stop the turn (past the 600 ms pair window)
  await expect(thread.getByTestId('runtime-working')).toBeVisible();
  await composer.press('Escape');
  await composer.press('Escape');
  await expect(thread.getByTestId('runtime-stopped')).toBeVisible({ timeout: 15_000 });
  await expect(stop).toHaveCount(0);
  await expect(composer).toHaveValue('next idea');
});

test('a question form answers in the dock with one Send in view', async ({ page }) => {
  const { id, thread, composer } = await openThread(page, 'Question dock');
  await composer.fill('question form');
  await composer.press('Enter');
  const dock = thread.getByTestId('runtime-attention');
  await expect(dock).toHaveAttribute('data-kind', 'question', { timeout: 15_000 });
  await expect(dock).toContainText('Codex asks');
  const send = thread.getByTestId('runtime-question-send');
  await expect(send).toHaveCount(1);
  await expectFullyInViewport(page, send, 'Send answer');
  expect(await onTop(page, send)).toBe(true);
  // The dock is capped to a share of its pane; the conversation keeps room.
  const [d, pane] = [(await dock.boundingBox())!, (await thread.boundingBox())!];
  expect(d.height).toBeLessThanOrEqual(pane.height * 0.46 + 1);
  await dock.getByLabel('Careful', { exact: false }).check();
  await dock.getByLabel('Your answer', { exact: true }).fill('Keep the API stable');
  await expectFullyInViewport(page, send, 'Send answer after filling');
  await shot(page, 'question-dock');
  const sent = page.waitForRequest((r) => r.method() === 'POST' && r.url().includes(`/runtime-sessions/${id}/requests/`));
  await send.click();
  await sent;
  await expect(dock).toHaveCount(0, { timeout: 15_000 });
  await expect(thread.getByText(/question answers:/)).toBeVisible({ timeout: 15_000 });
});

test('a failed turn is one calm block', async ({ page }) => {
  const { thread, composer } = await openThread(page, 'Failure block');
  await composer.fill('crash');
  await composer.press('Enter');
  const failed = thread.getByTestId('runtime-failed');
  await expect(failed).toHaveCount(1, { timeout: 20_000 });
  await expect(failed).toBeVisible();
  // Said once: no raw error row, no "0ms", no second "Agent stopped · Revive" line.
  await expect(thread.locator('[data-item-type="error"]')).toHaveCount(0);
  await expect(thread.getByTestId('runtime-closed')).toHaveCount(0);
  expect(await thread.locator('.rt-col').innerText()).not.toMatch(/\b0ms\b/);
  await expect(failed.getByTestId('runtime-turn-resend')).toHaveCount(1);
  await expect(failed.getByTestId('runtime-turn-edit')).toHaveCount(1);
  expect(await thread.getByTestId('runtime-turn-revive').count()).toBeLessThanOrEqual(1);
  await expect(failed).toHaveAttribute('data-cause', 'connection');
  await expect(failed).toContainText('Codex stopped unexpectedly');
  const details = failed.getByTestId('runtime-failed-details');
  if (await details.count()) {
    await details.click();
    await expect(failed.locator('.tf-detail')).toBeVisible();
  }
  await shot(page, 'failure');
  // Edit puts the message back in the composer.
  await composer.fill('');
  await failed.getByTestId('runtime-turn-edit').click();
  await expect(composer).toHaveValue('crash');
});
