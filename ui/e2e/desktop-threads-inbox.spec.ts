import { test, expect, devices, type APIRequestContext, type Page } from '@playwright/test';
import { execFileSync } from 'node:child_process';
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { apiCtx } from './seed';
import { expectFullyInViewport, expectNoHorizontalOverflow } from './helpers';

// ─────────────────────────────────────────────────────────────────────────────
// The Threads sidebar as an inbox (docs/features/threads.md "The sidebar"),
// driven through the isolated daemon's fake agent CLIs (global-setup shims
// codex → otto-fake-codex, claude → otto-fake-claude). The fakes answer by
// prompt: an echo, `ask` (a command approval), `question form` (a question),
// `fail` (a failed turn) and `hang` (runs until stopped).
// ─────────────────────────────────────────────────────────────────────────────

let ctx: APIRequestContext;
let base = '';
let wsId = '';
let root = '';
const ids: Record<string, string> = {};

async function waitStatus(id: string, ok: (r: Record<string, unknown>) => boolean): Promise<void> {
  let last: Record<string, unknown> = {};
  for (let i = 0; i < 120; i++) {
    last = (await (await ctx.get(`${base}/api/v1/runtime-sessions/${id}`)).json()) as Record<string, unknown>;
    if (ok(last)) return;
    await new Promise((r) => setTimeout(r, 250));
  }
  const ev = await (await ctx.get(`${base}/api/v1/runtime-sessions/${id}/events?after_sequence=0`)).text();
  throw new Error(`thread ${id} never reached the state: ${JSON.stringify({ status: last.status, last_turn_status: last.last_turn_status })} ${ev.slice(-1500)}`);
}

async function thread(key: string, provider: string, folder: string, title: string, prompt: string | null): Promise<string> {
  const r = await ctx.post(`${base}/api/v1/workspaces/${wsId}/runtime-sessions`, { data: { provider, cwd: join(root, folder), title, access_mode: 'approval-required' } });
  expect(r.ok(), await r.text()).toBeTruthy();
  const id = (await r.json()).id as string;
  ids[key] = id;
  if (prompt) await send(id, prompt);
  return id;
}

async function send(id: string, text: string): Promise<void> {
  const r = await ctx.post(`${base}/api/v1/runtime-sessions/${id}/prompt`, { data: { text } });
  expect(r.ok(), await r.text()).toBeTruthy();
}

// One seeded set of threads: a failure must not re-run beforeAll in a new worker.
test.describe.configure({ mode: 'serial' });

test.beforeEach(async ({}, info) => {
  test.skip(info.project.name !== 'desktop-browser', 'desktop-browser project only');
});

test.beforeAll(async () => {
  const c = await apiCtx();
  ctx = c.ctx;
  base = c.base;
  root = mkdtempSync(join(tmpdir(), 'otto-e2e-inbox-'));
  for (const f of ['harbor', 'harbor-api', 'fieldnotes', 'dotfiles']) {
    mkdirSync(join(root, f));
    writeFileSync(join(root, f, 'README.md'), `# ${f}\n`);
  }
  // Two of the projects are git repos, so their rows show a branch.
  for (const [f, branch] of [['harbor', 'feat/offline-sync'], ['harbor-api', 'feat/search-rate-limit']]) {
    const git = (...a: string[]) => execFileSync('git', ['-c', 'user.email=e2e@otto.test', '-c', 'user.name=e2e', ...a], { cwd: join(root, f) });
    git('init', '-q', '-b', branch);
    git('add', '.');
    git('commit', '-qm', 'init');
  }
  const r = await ctx.post(`${base}/api/v1/workspaces`, { data: { name: 'Inbox WS', root_path: root } });
  expect(r.ok(), await r.text()).toBeTruthy();
  wsId = (await r.json()).id as string;
  await thread('done', 'claude', 'fieldnotes', 'Rewrite the quickstart guide', 'hello there');
  await thread('idle', 'codex', 'harbor', 'Fix the flaky checkout test', 'hello');
  await thread('approval', 'codex', 'harbor-api', 'Rate limit the public search endpoint', 'ask');
  await thread('question', 'codex', 'harbor', 'Pick a sync strategy', 'question form');
  await thread('failed', 'codex', 'dotfiles', 'Speed up zsh startup', 'limit');
  await thread('working', 'codex', 'harbor', 'Dark mode for the notes editor', 'hang');
  await waitStatus(ids.approval, (s) => s.status === 'awaiting_permission');
  await waitStatus(ids.question, (s) => s.status === 'awaiting_permission');
  await waitStatus(ids.failed, (s) => s.last_turn_status === 'failed');
  await waitStatus(ids.working, (s) => s.status === 'working');
  await waitStatus(ids.idle, (s) => s.status === 'idle' && s.last_turn_status === 'completed');
});

test.afterAll(async () => {
  if (ctx && wsId) {
    const rows = (await (await ctx.get(`${base}/api/v1/workspaces/${wsId}/runtime-sessions`)).json()) as { id: string }[];
    for (const s of rows) await ctx.post(`${base}/api/v1/runtime-sessions/${s.id}/close`).catch(() => {});
  }
  await ctx?.dispose();
  if (root) rmSync(root, { recursive: true, force: true });
});

async function open(page: Page, focus: string, phone = false): Promise<void> {
  await page.addInitScript((id) => {
    if (sessionStorage.getItem('inbox-init')) return;
    sessionStorage.setItem('inbox-init', '1');
    localStorage.setItem('otto_workspace', id as string);
    localStorage.removeItem('otto_threads_v1');
  }, wsId);
  await page.goto(`/#/threads/${focus}`);
  if (!phone) await expect(page.getByTestId('threads-sidebar')).toBeVisible({ timeout: 30_000 });
  await expect(page.getByTestId(`runtime-thread-${focus}`)).toBeVisible({ timeout: 30_000 });
}

const row = (page: Page, key: string) => page.getByTestId(`thread-row-${ids[key]}`);
const word = (page: Page, key: string) => page.getByTestId(`thread-status-${ids[key]}`);

/** A turn that ends while another thread is in front leaves it unread ("Done"). */
async function finishDone(page: Page): Promise<void> {
  await send(ids.done, 'hello again');
  await expect(word(page, 'done')).toHaveText('Done', { timeout: 30_000 });
}

test('the sidebar is an inbox: status words, needs you first, Working and History folds', async ({ page }) => {
  await page.setViewportSize({ width: 1600, height: 1000 });
  await open(page, ids.idle);
  await finishDone(page);
  const side = page.getByTestId('threads-sidebar');
  const main = side.getByTestId('threads-active-list');

  // One word per row, the same vocabulary everywhere.
  await expect(word(page, 'approval')).toHaveText('Approval');
  await expect(word(page, 'question')).toHaveText('Question');
  await expect(word(page, 'failed')).toHaveText('Failed');
  await expect(word(page, 'idle')).toHaveText(/^(now|\d+[smhd])$/);

  // Needs you, then failed, then finished unread, then the rest — flat across projects.
  const order = await main.locator(':scope > [data-testid^="thread-row-"]').evaluateAll((els) => els.map((e) => e.getAttribute('data-testid')!.slice('thread-row-'.length)));
  const mine = order.filter((id) => Object.values(ids).includes(id)); // other specs' threads share the daemon
  expect(mine.slice(0, 2).sort()).toEqual([ids.approval, ids.question].sort());
  expect(mine.slice(2)).toEqual([ids.failed, ids.done, ids.idle]);

  // A row: project · status, title, branch · agent.
  await expect(row(page, 'approval').locator('.tr-proj')).toHaveText('harbor-api');
  await expect(row(page, 'approval').locator('.tr-branch')).toHaveText('feat/search-rate-limit', { timeout: 15_000 });
  await expect(row(page, 'approval').getByTestId('project-badge')).toHaveText('HA');

  // The working thread leaves the main list for "Working", saying for how long.
  const working = side.getByTestId('threads-working-list');
  await expect(main.getByTestId(`thread-row-${ids.working}`)).toHaveCount(0);
  await expect(working.getByTestId(`thread-row-${ids.working}`)).toBeVisible();
  await expect(word(page, 'working')).toHaveText(/^Working \d+[smh]/, { timeout: 15_000 });
  await side.getByTestId('threads-working-toggle').click();
  await expect(working.getByTestId(`thread-row-${ids.working}`)).toHaveCount(0);
  await side.getByTestId('threads-working-toggle').click();

  // History starts folded.
  await expect(side.getByTestId('threads-history-toggle')).toHaveAttribute('aria-expanded', 'false');

  // Opening the Done thread reads it: the word gives way to the time.
  await row(page, 'done').locator('.tr-main').click();
  await expect(word(page, 'done')).toHaveText(/^(now|\d+[smhd])$/);

  // ↑/↓ move between the rows.
  await row(page, 'done').locator('.tr-main').focus();
  await page.keyboard.press('ArrowUp');
  await expect(row(page, 'failed').locator('.tr-main')).toBeFocused();
});

test('Group by project brings back the folders and is remembered', async ({ page }) => {
  await page.setViewportSize({ width: 1600, height: 1000 });
  await open(page, ids.idle);
  const side = page.getByTestId('threads-sidebar');
  await side.getByTestId('threads-project-filter').click();
  await page.getByTestId('threads-group-by-project').click();
  await page.keyboard.press('Escape');
  const harbor = side.getByRole('region', { name: 'Project harbor', exact: true });
  await expect(harbor).toBeVisible();
  await expect(harbor.getByTestId(`thread-row-${ids.question}`)).toBeVisible();
  // Working threads stay in their fold, not in the project.
  await expect(harbor.getByTestId(`thread-row-${ids.working}`)).toHaveCount(0);
  await page.reload();
  await expect(side.getByRole('region', { name: 'Project harbor', exact: true })).toBeVisible({ timeout: 30_000 });
  await side.getByTestId('threads-project-filter').click();
  await page.getByTestId('threads-group-by-project').click();
  await page.keyboard.press('Escape');
  await expect(side.getByRole('region', { name: 'Project harbor', exact: true })).toHaveCount(0);
});

test('pin and Move to History act at once with one Undo notice; ⌘Z undoes outside a text field', async ({ page }) => {
  await page.setViewportSize({ width: 1600, height: 1000 });
  await open(page, ids.idle);
  const side = page.getByTestId('threads-sidebar');
  const undo = page.locator('.toast', { hasText: /Pinned|History|Active/ }).locator('.toast-action');

  // Pin: no question, the row moves to Pinned, Undo puts it back.
  await row(page, 'failed').hover();
  await page.getByTestId(`thread-pin-${ids.failed}`).click();
  await expect(side.getByTestId(`thread-pinned-${ids.failed}`)).toBeVisible();
  await expect(page.locator('.toast', { hasText: 'Pinned' })).toBeVisible();
  await undo.click();
  await expect(side.getByTestId(`thread-pinned-${ids.failed}`)).toHaveCount(0);
  await expect(row(page, 'failed')).toBeVisible();

  // Move to History from the row's menu; a second action replaces the notice.
  await row(page, 'idle').locator('.tr-main').click({ button: 'right' });
  await page.locator('.ctx-menu').getByRole('menuitem', { name: 'Move to History' }).click();
  await expect(side.getByTestId('threads-active-list').getByTestId(`thread-row-${ids.idle}`)).toHaveCount(0);
  await expect(page.locator('.toast', { hasText: 'Moved to History' })).toBeVisible();
  await expect(page.locator('.toast', { hasText: 'Pinned' })).toHaveCount(0);

  // ⌘Z inside the composer is the field's own undo …
  await page.getByTestId(`runtime-thread-${ids.idle}`).getByTestId('runtime-composer').focus();
  await page.keyboard.press('Meta+z');
  await expect(page.locator('.toast', { hasText: 'Moved to History' })).toBeVisible();
  // … elsewhere it is the notice's Undo.
  await side.getByTestId('threads-search').focus();
  await page.locator('body').click({ position: { x: 900, y: 400 } });
  await page.keyboard.press('Meta+z');
  await expect(side.getByTestId('threads-active-list').getByTestId(`thread-row-${ids.idle}`)).toBeVisible();
  await expect(page.locator('.toast', { hasText: 'Moved to History' })).toHaveCount(0);

  // Dismissing a side chat acts at once; Undo brings it back.
  const composer = page.getByTestId(`runtime-thread-${ids.idle}`).getByTestId('runtime-composer');
  await composer.fill('/btw what changed so far?');
  await composer.press('Enter');
  const sideChat = page.getByTestId('threads-side-chat');
  await expect(sideChat.getByTestId('side-chat-dismiss')).toBeVisible({ timeout: 15_000 });
  await sideChat.getByTestId('side-chat-dismiss').click();
  await expect(sideChat.getByTestId('side-chat-dismiss')).toHaveCount(0);
  await page.locator('.toast', { hasText: 'Side chat dismissed' }).locator('.toast-action').click();
  await expect(sideChat.getByTestId('side-chat-dismiss')).toBeVisible({ timeout: 15_000 });
});

const SHOTS = process.env.OTTO_SHOTS_DIR ?? '';

test('screenshots: the sidebar, light and dark, desktop and phone', async ({ page, browser }, info) => {
  test.skip(!SHOTS, 'set OTTO_SHOTS_DIR to take screenshots');
  const tag = process.env.OTTO_SHOTS_TAG ?? 'after';
  await page.setViewportSize({ width: 1600, height: 1000 });
  await open(page, ids.idle);
  await finishDone(page).catch(() => {});
  for (const scheme of ['dark', 'light'] as const) {
    await page.emulateMedia({ colorScheme: scheme });
    await page.screenshot({ path: join(SHOTS, `${tag}-desktop-${scheme}.png`) });
  }
  const phone = await browser.newContext({ ...devices['iPhone 14'], storageState: info.project.use.storageState as string, baseURL: info.project.use.baseURL });
  const p = await phone.newPage();
  await open(p, ids.idle, true);
  await p.getByTestId('threads-toggle-sidebar').click();
  await expectFullyInViewport(p, p.getByTestId('threads-sidebar'), 'threads sidebar drawer');
  await expectNoHorizontalOverflow(p);
  for (const scheme of ['dark', 'light'] as const) {
    await p.emulateMedia({ colorScheme: scheme });
    await p.screenshot({ path: join(SHOTS, `${tag}-phone-${scheme}.png`) });
  }
  await phone.close();
});
