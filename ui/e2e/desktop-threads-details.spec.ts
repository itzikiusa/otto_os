import { test, expect, type APIRequestContext, type Page } from '@playwright/test';
import { mkdtempSync, rmSync, writeFileSync } from 'node:fs';
import { execFileSync } from 'node:child_process';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { apiCtx } from './seed';
import { expectFullyInViewport, expectNoHorizontalOverflow } from './helpers';

// ─────────────────────────────────────────────────────────────────────────────
// Thread details (ThreadDetailsCard.svelte): ONE card for a thread's facts —
// folder (Open in ▾), branch ▾, the one git action the checkout is ready for,
// Changes +N −M, then its subagents. It floats top-right beside the
// conversation while the right panel is closed and a whole reading column is
// left (the column moves over, nothing is covered), heads the right panel
// otherwise, and is dismissible (remembered) with ⋯ › View bringing it back.
// Real structured threads on the isolated daemon: Claude via otto-fake-claude
// (`agent <s>` runs a background subagent), Codex via otto-fake-codex.
// ─────────────────────────────────────────────────────────────────────────────

let ctx: APIRequestContext;
let base = '';
let wsId = '';
let dir = '';
const git = (...a: string[]): string => execFileSync('git', ['-c', 'user.email=e2e@otto', '-c', 'user.name=e2e', ...a], { cwd: dir }).toString();

test.beforeAll(async () => {
  const c = await apiCtx();
  ctx = c.ctx;
  base = c.base;
  dir = mkdtempSync(join(tmpdir(), 'harbor-'));
  writeFileSync(join(dir, 'README.md'), '# harbor\n');
  git('init', '-q', '-b', 'main');
  git('config', 'user.email', 'e2e@otto');
  git('config', 'user.name', 'e2e');
  git('add', '.');
  git('commit', '-qm', 'init');
  git('checkout', '-qb', 'feat/offline-sync');
  writeFileSync(join(dir, 'queue.ts'), 'export const queue = [];\nexport const drain = () => queue.shift();\n');
  const r = await ctx.post(`${base}/api/v1/workspaces`, { data: { name: 'Harbor', root_path: dir } });
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
    if (sessionStorage.getItem('details-init')) return;
    sessionStorage.setItem('details-init', '1');
    localStorage.setItem('otto_workspace', id as string);
    localStorage.removeItem('otto_threads_v1');
    localStorage.removeItem('otto_threads_chrome_v1');
  }, wsId);
});

async function thread(provider: 'claude' | 'codex', title: string): Promise<string> {
  const created = await ctx.post(`${base}/api/v1/workspaces/${wsId}/runtime-sessions`, { data: { provider, cwd: dir, title } });
  expect(created.ok(), await created.text()).toBeTruthy();
  return (await created.json()).id as string;
}

const overlaps = (a: { x: number; y: number; width: number; height: number }, b: typeof a): boolean =>
  a.x < b.x + b.width && b.x < a.x + a.width && a.y < b.y + b.height && b.y < a.y + a.height;

async function shots(page: Page, name: string): Promise<void> {
  const dirOut = process.env.THRC_SHOTS;
  for (const scheme of ['light', 'dark'] as const) {
    await page.emulateMedia({ colorScheme: scheme });
    await page.screenshot({ path: dirOut ? `${dirOut}/${name}-${scheme}.png` : test.info().outputPath(`${name}-${scheme}.png`), animations: 'disabled' });
  }
  await page.emulateMedia({ colorScheme: 'light' });
}

test('the details card floats beside a Claude thread, heads the right panel, and lists its subagents', async ({ page }) => {
  test.setTimeout(150_000);
  await page.setViewportSize({ width: 1600, height: 1000 });
  const id = await thread('claude', 'Sync offline edits on reconnect');
  await page.goto(`/#/threads/${id}`);
  const conv = page.getByTestId(`runtime-thread-${id}`);
  await expect(conv).toBeVisible({ timeout: 30_000 });

  // Top bar: project badge + "project / title" in reading type.
  const crumbs = page.getByTestId('threads-breadcrumb');
  await expect(crumbs.getByTestId('project-badge')).toHaveText(/^[A-Z0-9]{2}$/);
  await expect(crumbs).toContainText('Sync offline edits on reconnect');
  expect(await crumbs.evaluate((el) => getComputedStyle(el).fontSize)).toBe('15px');

  // The card: folder, branch, the git action (a dirty checkout → Commit), Changes.
  const card = page.getByTestId('thread-details-card');
  await expect(card).toBeVisible();
  await expect(card.getByTestId('details-folder')).toContainText('harbor-');
  await expect(card.getByTestId('details-branch')).toContainText('feat/offline-sync', { timeout: 15_000 });
  await expect(card.getByTestId('details-git-action')).toHaveAttribute('data-kind', 'commit');
  await expect(card.getByTestId('details-changes')).toContainText('+2');
  await expectFullyInViewport(page, card, 'details card');
  expect(await page.locator('.th-stage').evaluate((el) => getComputedStyle(el).getPropertyValue('--thread-card-reserve').trim())).toBe('296px');

  // Open in ▾ carries the project scripts (moved off the card).
  await card.getByTestId('details-open-in').click();
  const menu = page.locator('.ctx-menu');
  await expectFullyInViewport(page, menu, 'Open in menu');
  for (const name of ['Files', 'Git', 'Terminal', 'Copy path', 'Add project script…']) await expect(menu.getByRole('menuitem', { name })).toBeVisible();
  await page.keyboard.press('Escape');

  // A running background subagent shows under "Subagents · 1 working" with its time.
  const composer = conv.getByTestId('runtime-composer');
  await composer.fill('agent 120');
  await composer.press('Enter');
  await expect(conv.getByText("I'm waiting for the PR sweep to come back.")).toBeVisible({ timeout: 20_000 });
  const subs = card.getByTestId('details-subagents');
  await expect(subs).toContainText('Subagents · 1 working');
  const sub = subs.getByTestId('details-subagent');
  await expect(sub).toHaveAttribute('data-word', 'Working');
  await expect(sub).toContainText(/PR sweep\s*Working\s*\d+s/);

  // Nothing in the conversation runs under the card.
  const cardBox = (await card.boundingBox())!;
  for (const el of [conv.getByText('agent 120', { exact: true }), conv.getByText("I'm waiting for the PR sweep to come back."), composer]) {
    expect(overlaps((await el.boundingBox())!, cardBox), 'a message runs under the card').toBe(false);
  }
  await shots(page, 'details-desktop');

  // A turn's own subagent has no thread of its own (its row is in the
  // timeline): its card row opens the right panel the card now heads (flat).
  await expect(sub.getByTestId('agents-tasks-stop')).toBeVisible();
  await sub.getByRole('button', { name: /^PR sweep Working/ }).click();
  const panel = page.getByTestId('threads-inspector');
  await expect(page.getByTestId('thread-details-card')).toHaveCount(1);
  await expect(panel.getByTestId('thread-details-card')).toBeVisible();
  await expect(panel.getByTestId('details-subagent')).toContainText('PR sweep');
  expect(await page.locator('.th-stage').evaluate((el) => getComputedStyle(el).getPropertyValue('--thread-card-reserve').trim())).toBe('0px');
  await shots(page, 'details-panel');

  // Changes opens the Changes surface: the card folds to its one summary row
  // and the panel opens wide enough for a diff, the conversation beside it.
  await panel.getByTestId('details-changes').click();
  await expect(page.getByTestId('threads-tab-changes')).toBeVisible();
  await expect(panel.getByTestId('details-summary')).toBeVisible();
  await expect(panel.getByTestId('details-folder')).toHaveCount(0);
  expect((await panel.boundingBox())!.width).toBeGreaterThanOrEqual(480);
  expect((await conv.boundingBox())!.width).toBeGreaterThanOrEqual(600);
  await page.getByTestId('threads-toggle-inspector').click();
  await expect(page.getByTestId('thread-details-card')).toBeVisible();

  // Commit over the thread: Git's changes panel in a sheet, everything staged,
  // the message, Commit. The action then moves on — with no remote there is
  // no next step, so the row goes (never a dead row) and Changes says "No changes".
  await card.getByTestId('details-git-action').click();
  const sheet = page.getByTestId('thread-commit-sheet');
  await expect(sheet).toBeVisible();
  await expect(page.getByRole('dialog')).toContainText('Commit to feat/offline-sync');
  await expect(sheet.getByText('Nothing staged yet.')).toHaveCount(0);
  await expect(conv).toBeAttached();
  await shots(page, 'details-commit-sheet');
  await sheet.getByLabel('Commit summary').fill('Queue offline edits');
  await sheet.locator('.wc-commit').click();
  await expect(sheet).toHaveCount(0, { timeout: 20_000 });
  expect(git('log', '-1', '--format=%s').trim()).toBe('Queue offline edits');
  await expect(card.getByTestId('details-git-action')).toHaveCount(0, { timeout: 20_000 });
  await expect(card.getByTestId('details-changes')).toHaveText('No changes');

  // Dismissed: gone and remembered; ⋯ › View › Thread details brings it back.
  await card.getByTestId('details-dismiss').click();
  await expect(page.getByTestId('thread-details-card')).toHaveCount(0);
  expect(await page.locator('.th-stage').evaluate((el) => getComputedStyle(el).getPropertyValue('--thread-card-reserve').trim())).toBe('0px');
  await page.reload();
  await expect(conv).toBeVisible({ timeout: 30_000 });
  await expect(page.getByTestId('thread-details-card')).toHaveCount(0);
  await page.getByTestId('threads-thread-menu').click();
  await page.locator('.ctx-menu').getByRole('menuitem', { name: 'View' }).click();
  await page.locator('.ctx-submenu').getByRole('menuitemcheckbox', { name: 'Thread details' }).click();
  await expect(page.getByTestId('thread-details-card')).toBeVisible();

  // No room beside a reading column: the card does not float (it heads the panel).
  await page.setViewportSize({ width: 1300, height: 900 });
  await expect(page.getByTestId('thread-details-card')).toHaveCount(0);
  await page.getByTestId('threads-toggle-inspector').click();
  await expect(page.getByTestId('threads-inspector').getByTestId('thread-details-card')).toBeVisible();
  await page.getByTestId('threads-toggle-inspector').click();

  // Phone: never floating; the details drawer heads with it; no sideways scroll.
  await page.setViewportSize({ width: 390, height: 844 });
  await expect(page.getByTestId('thread-details-card')).toHaveCount(0);
  await expectNoHorizontalOverflow(page);
  await page.getByTestId('threads-toggle-inspector').click();
  const drawerCard = page.getByTestId('thread-details-card');
  await expect(drawerCard).toBeVisible();
  await expectFullyInViewport(page, drawerCard, 'details card (phone drawer)');
  await expectNoHorizontalOverflow(page);
  await shots(page, 'details-phone');
});

test('a Codex thread gets the same card; a thread with no subagents shows no Subagents section', async ({ page }) => {
  await page.setViewportSize({ width: 1600, height: 1000 });
  const id = await thread('codex', 'Codex details');
  await page.goto(`/#/threads/${id}`);
  const conv = page.getByTestId(`runtime-thread-${id}`);
  await expect(conv).toBeVisible({ timeout: 30_000 });
  const composer = conv.getByTestId('runtime-composer');
  await composer.fill('hello');
  await composer.press('Enter');
  await expect(conv.getByText('hello').last()).toBeVisible({ timeout: 20_000 });
  const card = page.getByTestId('thread-details-card');
  await expect(card).toBeVisible();
  await expect(card.getByTestId('details-branch')).toContainText('feat/offline-sync', { timeout: 15_000 });
  await expect(card.getByTestId('details-subagents')).toHaveCount(0);
  // The new-thread stage floats the same card for the draft's folder.
  await page.goto('/#/threads/new');
  await expect(page.getByTestId('new-thread-stage')).toBeVisible({ timeout: 30_000 });
  await expect(page.getByTestId('thread-details-card')).toBeVisible();
  await shots(page, 'details-stage');
});
