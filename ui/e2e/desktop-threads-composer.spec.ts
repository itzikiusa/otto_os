import { test, expect, type APIRequestContext, type Locator, type Page } from '@playwright/test';
import { mkdtempSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { apiCtx } from './seed';
import { expectFullyInViewport, expectNoHorizontalOverflow } from './helpers';

// ─────────────────────────────────────────────────────────────────────────────
// The Threads composer and timeline on REAL fake-agent threads: Codex runs
// otto-fake-codex (`codex app-server`), Claude otto-fake-claude (stream-json);
// both answer `slow <s>` with "Working…" and take a steer meanwhile
// ("steered: <text>"). Needs OTTO_E2E_BIN on a build with the fakes beside it.
//   • ONE primary button that says what it will do: Send idle, Stop while the
//     agent works with nothing typed, Queue (default) / Steer with a draft;
//     ⌘↵ the other one; the choice in the toolbar ⋯ › View (composerSend.ts).
// ─────────────────────────────────────────────────────────────────────────────

test.use({ serviceWorkers: 'block' });

let ctx: APIRequestContext;
let base = '';
let wsId = '';
let fixtureDir = '';

test.beforeAll(async () => {
  const c = await apiCtx();
  ctx = c.ctx;
  base = c.base;
  fixtureDir = mkdtempSync(join(tmpdir(), 'otto-e2e-threads-composer-'));
  writeFileSync(join(fixtureDir, 'README.md'), '# Threads composer\n');
  const r = await ctx.post(`${base}/api/v1/workspaces`, { data: { name: 'Composer WS', root_path: fixtureDir } });
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
    localStorage.setItem('otto_workspace', id as string);
    localStorage.removeItem('otto_threads_prefs_v1');
  }, wsId);
  await page.setViewportSize({ width: 1440, height: 900 });
});

async function open(page: Page, provider: 'codex' | 'claude', title: string): Promise<{ id: string; thread: Locator }> {
  const r = await ctx.post(`${base}/api/v1/workspaces/${wsId}/runtime-sessions`, { data: { provider, cwd: fixtureDir, title } });
  expect(r.ok(), await r.text()).toBeTruthy();
  const id = (await r.json()).id as string;
  await page.goto(`/#/threads/${id}`);
  const thread = page.getByTestId(`runtime-thread-${id}`);
  await expect(thread).toBeVisible({ timeout: 30_000 });
  return { id, thread };
}

async function send(thread: Locator, text: string, reply = `echo: ${text}`): Promise<void> {
  await thread.getByTestId('runtime-composer').fill(text);
  await thread.getByTestId('runtime-send').click();
  await expect(thread.getByText(reply, { exact: false }).last()).toBeVisible({ timeout: 20_000 });
}

/** Start a turn that runs ~`secs` and wait until the agent is working. */
async function startSlow(thread: Locator, secs: number): Promise<void> {
  await thread.getByTestId('runtime-composer').fill(`slow ${secs}`);
  await thread.getByTestId('runtime-send').click();
  await expect(thread.getByText('Working…').last()).toBeVisible({ timeout: 20_000 });
}

async function pickView(page: Page, name: string): Promise<void> {
  await page.getByTestId('threads-thread-menu').click();
  await page.locator('.ctx-menu').getByRole('menuitem', { name: 'View' }).click();
  await page.locator('.ctx-submenu').getByRole('menuitemcheckbox', { name }).click();
}

for (const provider of ['codex', 'claude'] as const) {
  test(`${provider}: one primary button — Stop, then Queue or Steer with a draft; ⌘↵ the other; the View choice`, async ({ page }) => {
    test.setTimeout(150_000);
    const { thread } = await open(page, provider, `Primary ${provider}`);
    const composer = thread.getByTestId('runtime-composer');
    const primary = thread.getByTestId('runtime-send');
    const stop = thread.getByTestId('runtime-stop');
    await expect(primary).toHaveAttribute('data-delivery', 'send');
    await expect(stop).toHaveCount(0);

    // Working with nothing typed: Stop is the one button.
    await startSlow(thread, 12);
    await expect(stop).toBeVisible();
    await expect(primary).toHaveCount(0);
    // A draft: the primary names the default delivery, Stop steps back.
    await composer.fill('queued one');
    await expect(primary).toHaveAttribute('data-delivery', 'queue');
    await expect(primary).toHaveAttribute('aria-label', 'Queue — sends when this turn ends');
    await expect(primary).toHaveAttribute('title', /⌘↵ steers/);
    await expect(stop).toBeVisible();
    const [p, s] = [await primary.boundingBox(), await stop.boundingBox()];
    expect(s!.width).toBeLessThan(p!.width);
    await composer.press('Enter');
    await expect(thread.getByTestId('runtime-queued')).toHaveCount(1);
    // ⌘↵ steers into the running turn (the queue bar's path).
    await composer.fill('also check b');
    await composer.press('Meta+Enter');
    await expect(thread.getByText('steered: also check b').last()).toBeVisible({ timeout: 20_000 });
    await expect(thread.getByTestId('runtime-queued')).toHaveCount(1);

    // ⋯ › View: Enter steers from now on (per device), ⌘↵ queues.
    await pickView(page, 'While working, messages steer');
    await composer.fill('now steer');
    await expect(primary).toHaveAttribute('data-delivery', 'steer');
    await expect(primary).toHaveAttribute('title', /⌘↵ queues/);
    await expect(composer).toHaveAttribute('placeholder', /./);
    await composer.press('Enter');
    await expect(thread.getByText('steered: now steer').last()).toBeVisible({ timeout: 20_000 });
    await composer.fill('then queue');
    await composer.press('Meta+Enter');
    await expect(thread.getByTestId('runtime-queued')).toHaveCount(2);
    expect(await page.evaluate(() => localStorage.getItem('otto_threads_prefs_v1'))).toContain('steer');
    // Stop from the small control ends the turn; the queue then drains.
    await stop.click();
    await expect(thread.getByText('echo: queued one').last()).toBeVisible({ timeout: 30_000 });
    await expect(thread.getByText('echo: then queue').last()).toBeVisible({ timeout: 30_000 });
    await expect(primary).toHaveAttribute('data-delivery', 'send');
    await expectNoHorizontalOverflow(page);
  });
}
