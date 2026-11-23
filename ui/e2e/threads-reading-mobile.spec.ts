import { test, expect, type APIRequestContext, type Locator, type Page } from '@playwright/test';
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { apiCtx, seedWorkspace } from './seed';
import { expectFullyInViewport, expectNoHorizontalOverflow } from './helpers';
import { ITEMS, longThread, mockThread, TURNS } from './threadsReadingFixture';

// ─────────────────────────────────────────────────────────────────────────────
// The Threads composer and reading view on phones and tablets, with real
// touch (tap): the one primary button (Stop / Queue / Steer) on a REAL
// fake-codex thread with the keyboard up, the "+" menu, the per-device
// "while working" and tool-call choices in ⋯ › View, and a finished thread's
// one-line fold, changes card and always-visible times (no hover on touch).
// OTTO_E2E_SHOTS=<dir> keeps light/dark screenshots.
// ─────────────────────────────────────────────────────────────────────────────

test.use({ serviceWorkers: 'block' });
const MOBILE = /^(iphone-|ipad-|android-)/;
const SHOTS = process.env.OTTO_E2E_SHOTS ?? '';

let ctx: APIRequestContext;
let base = '';
let wsId = '';
let mockWs = '';
let fixtureDir = '';

test.beforeAll(async () => {
  const c = await apiCtx();
  ctx = c.ctx;
  base = c.base;
  fixtureDir = mkdtempSync(join(tmpdir(), 'otto-e2e-threads-reading-m-'));
  writeFileSync(join(fixtureDir, 'README.md'), '# Phone composer\n');
  const r = await ctx.post(`${base}/api/v1/workspaces`, { data: { name: 'Phone Composer WS', root_path: fixtureDir } });
  expect(r.ok(), await r.text()).toBeTruthy();
  wsId = (await r.json()).id as string;
  mockWs = await seedWorkspace(ctx, base, 'Phone Reading WS');
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
  test.skip(!MOBILE.test(info.project.name), 'phones and tablets');
  await page.addInitScript(() => localStorage.removeItem('otto_threads_prefs_v1'));
});

async function shoot(page: Page, name: string, project: string): Promise<void> {
  if (!SHOTS) return;
  mkdirSync(SHOTS, { recursive: true });
  for (const scheme of ['light', 'dark'] as const) {
    await page.emulateMedia({ colorScheme: scheme });
    await page.screenshot({ path: `${SHOTS}/${project}-${name}-${scheme}.png`, animations: 'disabled' });
  }
  await page.emulateMedia({ colorScheme: 'light' });
}

/** What the on-screen keyboard leaves (threads-phone-layout.spec.ts). */
function withKeyboard(page: Page): { width: number; height: number } {
  const { width: w, height: h } = page.viewportSize()!;
  return { width: w, height: w > h ? h - 180 : Math.round(h * 0.55) };
}

async function atLeast44(el: Locator, what: string): Promise<void> {
  const b = (await el.boundingBox())!;
  expect(Math.min(b.width, b.height), `${what} touch target`).toBeGreaterThanOrEqual(43.5);
}

async function pickView(page: Page, name: string): Promise<void> {
  await page.getByTestId('threads-thread-menu').tap();
  await page.locator('.ctx-menu').getByRole('menuitem', { name: 'View' }).tap();
  const item = page.locator('.ctx-submenu, .ctx-menu').getByRole('menuitemcheckbox', { name });
  await item.first().scrollIntoViewIfNeeded(); // a short screen: the capped flyout scrolls
  await expectFullyInViewport(page, item.first(), name);
  await item.first().tap();
}

test('phone: one primary button by touch — Stop, Queue, Steer — with the keyboard up; "+" by touch', async ({ page }, info) => {
  test.setTimeout(120_000);
  await page.addInitScript((id) => localStorage.setItem('otto_workspace', id as string), wsId);
  const r = await ctx.post(`${base}/api/v1/workspaces/${wsId}/runtime-sessions`, { data: { provider: 'codex', cwd: fixtureDir, title: `Phone primary ${info.project.name}` } });
  const id = (await r.json()).id as string;
  await page.goto(`/#/threads/${id}`);
  const thread = page.getByTestId(`runtime-thread-${id}`);
  await expect(thread).toBeVisible({ timeout: 30_000 });
  const composer = thread.getByTestId('runtime-composer');
  const primary = thread.getByTestId('runtime-send');
  const stop = thread.getByTestId('runtime-stop');
  const plus = thread.getByTestId('runtime-plus');
  await atLeast44(primary, 'Send');
  await atLeast44(plus, '+');

  // "+" by touch: every entry on screen; Mention types "@".
  await plus.tap();
  const menu = page.locator('.ctx-menu');
  for (const name of [/Attach an image/, /Mention a file or agent/, /Access:/]) await expectFullyInViewport(page, menu.getByRole('menuitem').or(menu.getByRole('menuitemcheckbox')).filter({ hasText: name }).first(), String(name));
  await menu.getByRole('menuitem', { name: /Mention a file or agent/ }).tap();
  await expect(composer).toHaveValue('@');
  await composer.fill('');

  await composer.tap();
  await composer.fill('slow 10');
  await primary.tap();
  await expect(thread.getByText('Working…').last()).toBeVisible({ timeout: 20_000 });
  await expect(primary).toHaveCount(0);
  await atLeast44(stop, 'Stop');
  // The keyboard up (as on a device: focus first, then the keyboard takes
  // its share of the screen): the composer and its button stay on screen.
  await composer.tap();
  await page.setViewportSize(withKeyboard(page));
  await expect(page.locator('html[data-kb="open"]')).toHaveCount(1);
  await composer.fill('queued from the phone');
  await expect(primary).toHaveAttribute('data-delivery', 'queue');
  await expectFullyInViewport(page, primary, 'Queue');
  await expectFullyInViewport(page, stop, 'small Stop');
  await atLeast44(stop, 'secondary Stop');
  await primary.tap();
  // With the keyboard up the queue strip folds into the one notice line, the
  // input is one row and the latest message stays in sight.
  const more = thread.getByTestId('runtime-banners-more');
  await expect(more).toContainText('queue');
  await expect(thread.getByTestId('runtime-queued')).toHaveCount(0);
  const lineH = await composer.evaluate((el) => parseFloat(getComputedStyle(el).lineHeight));
  // A short pane (a phone with the keyboard up) gets a one-row input; a tablet keeps two rows.
  const paneH = (await thread.boundingBox())!.height;
  if (paneH <= 420) expect((await composer.boundingBox())!.height, 'one-row input in a short pane').toBeLessThan(lineH * 1.6);
  const [sb, last] = [(await thread.locator('.rt-scroll').boundingBox())!, (await thread.getByText('Working…').last().boundingBox())!];
  expect(last.y + last.height, 'the latest message is in sight').toBeLessThanOrEqual(sb.y + sb.height + 1);
  expect(last.y, 'the latest message is in sight').toBeGreaterThanOrEqual(sb.y - 1);
  await more.tap();
  await expect(thread.getByTestId('runtime-queued')).toHaveCount(1);
  await shoot(page, 'queue', info.project.name);

  // ⋯ › View › While working, messages steer — by touch.
  await pickView(page, 'While working, messages steer');
  await composer.tap();
  await composer.fill('steer from the phone');
  await expect(primary).toHaveAttribute('data-delivery', 'steer');
  await primary.tap();
  await expect(thread.getByText('steered: steer from the phone').last()).toBeVisible({ timeout: 20_000 });
  await stop.tap();
  await expect(thread.getByText('echo: queued from the phone').last()).toBeVisible({ timeout: 30_000 });
  await expectNoHorizontalOverflow(page);
});

test('phone: a finished turn reads as one line and a changes card; times show without hover; tool calls hide by touch', async ({ page }, info) => {
  await mockThread(page, mockWs, 'phone-read', ITEMS, TURNS);
  await page.goto('/#/threads/phone-read');
  const thread = page.getByTestId('runtime-thread-phone-read');
  await expect(thread.getByText('Offline edits now survive')).toBeVisible({ timeout: 30_000 });
  // No hover on touch: the time is always shown.
  await expect(thread.getByTestId('runtime-assistant-time')).toHaveCSS('opacity', '1');
  const fold = thread.locator('[data-testid^="runtime-fold-"] > summary');
  await expect(fold).toContainText('ran 4 commands and edited 3 files');
  const card = thread.getByTestId('runtime-turn-changes');
  const toggle = card.getByRole('button', { name: /3 changed files/ });
  await atLeast44(toggle, 'changes card');
  await expect(card.locator('li')).toHaveCount(3); // ≤ 5 files: open
  await atLeast44(card.locator('li button').first(), 'a changed file');
  await toggle.tap();
  await expect(card.locator('li')).toHaveCount(0);
  await toggle.tap();
  await expect(card.locator('li')).toHaveCount(3);
  await expectNoHorizontalOverflow(page);
  await shoot(page, 'reading', info.project.name);
  // The keyboard up: the composer stays on screen and nothing of the thread sits under it.
  const full = page.viewportSize()!;
  await page.setViewportSize(withKeyboard(page));
  const composer = thread.getByTestId('runtime-composer');
  await composer.tap();
  await expectFullyInViewport(page, composer, 'composer with the keyboard up');
  // The conversation (changes card included) scrolls above the composer, never under it.
  const [sb, kb] = [(await thread.locator('.rt-scroll').boundingBox())!, (await thread.locator('.rt-composer').boundingBox())!];
  expect(sb.y + sb.height, 'the conversation ends above the composer').toBeLessThanOrEqual(kb.y + 1);
  // A phone on its side with the keyboard up leaves ~250 px: the conversation keeps a strip.
  expect(sb.height, 'some conversation stays readable').toBeGreaterThan(page.viewportSize()!.height > 300 ? 120 : 16);
  await card.scrollIntoViewIfNeeded();
  const cb = (await card.boundingBox())!;
  expect(cb.y, 'the changes card scrolls into the visible conversation').toBeLessThan(sb.y + sb.height);
  await shoot(page, 'reading-keyboard', info.project.name);
  await page.setViewportSize(full);
  await fold.tap();
  await expect(thread.locator('[data-item-type="command_execution"]').first()).toBeVisible();
  await pickView(page, 'Tool calls: hide all');
  await expect(thread.locator('[data-item-type="command_execution"]')).toHaveCount(0);
  await expect(thread.getByText('Offline edits now survive')).toBeVisible();
  await expect(card).toBeVisible();
});

test('phone: Settings › Threads by touch', async ({ page }, info) => {
  await page.goto('/#/settings/threads');
  const sec = page.getByTestId('settings-threads');
  await expect(sec).toBeVisible({ timeout: 30_000 });
  const steer = sec.getByRole('radiogroup', { name: 'While the agent works, Enter' }).getByRole('radio', { name: 'Steer' });
  await atLeast44(steer, 'Steer option');
  await steer.tap();
  await expect(steer).toHaveAttribute('aria-checked', 'true');
  await sec.getByRole('radiogroup', { name: 'Tool calls' }).getByRole('radio', { name: 'Hide all' }).tap();
  expect(JSON.parse((await page.evaluate(() => localStorage.getItem('otto_threads_prefs_v1'))) ?? '{}')).toMatchObject({ midTurn: 'steer', tools: 'none' });
  await expectNoHorizontalOverflow(page);
  await shoot(page, 'settings-threads', info.project.name);
});

test('phone: a long thread’s turns are a "Jump to…" sheet in ⋯ (no rail)', async ({ page }, info) => {
  const { items, turns } = longThread(10);
  await mockThread(page, mockWs, 'phone-long', items, turns);
  await page.goto('/#/threads/phone-long');
  const thread = page.getByTestId('runtime-thread-phone-long');
  await expect(thread.getByText('Answer 10.')).toBeVisible({ timeout: 30_000 });
  if (/^iphone-|^android-phone/.test(info.project.name)) await expect(thread.getByTestId('runtime-minimap')).toHaveCount(0);
  await page.getByTestId('threads-thread-menu').tap();
  await page.locator('.ctx-menu').getByRole('menuitem', { name: 'Jump to…' }).tap();
  const list = page.getByTestId('runtime-jump-list');
  await expect(list.getByRole('button')).toHaveCount(10);
  await expect(list.getByRole('button').nth(2)).toContainText('Failed');
  await expect(list.getByRole('button').nth(4)).toContainText('Approval');
  await atLeast44(list.getByRole('button').first(), 'a turn in the jump list');
  await expectNoHorizontalOverflow(page);
  await shoot(page, 'jump-sheet', info.project.name);
  await list.getByRole('button', { name: /Question number 2:/ }).tap();
  await expect(list).toHaveCount(0);
  await expect(thread.locator('[data-rt-msg="u2"]')).toBeInViewport();
});
