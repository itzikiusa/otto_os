import { test, expect, type APIRequestContext, type Locator, type Page } from '@playwright/test';
import { mkdtempSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { apiCtx } from './seed';
import { expectFullyInViewport, expectNoHorizontalOverflow } from './helpers';

// ─────────────────────────────────────────────────────────────────────────────
// "Continue with another provider" on REAL fake-agent threads (otto-fake-claude
// / otto-fake-codex echo the prompt they read, so the reply shows the handoff
// the new provider got). The model picker lists the thread's provider, then a
// divider and the others; a model there ARMS the switch ("Next message
// continues on Codex · Cancel"), nothing is sent. The next message continues
// the SAME thread on the new provider, after a "Context handoff · A → B"
// divider that opens on what was handed over. Back again is another handoff.
// ─────────────────────────────────────────────────────────────────────────────

test.use({ serviceWorkers: 'block' });

/** Where screenshots go (OTTO_E2E_SHOTS), if anywhere. */
const SHOTS = process.env.OTTO_E2E_SHOTS ?? '';

let ctx: APIRequestContext;
let base = '';
let wsId = '';
let fixtureDir = '';

test.beforeAll(async () => {
  const c = await apiCtx();
  ctx = c.ctx;
  base = c.base;
  fixtureDir = mkdtempSync(join(tmpdir(), 'otto-e2e-provider-switch-'));
  writeFileSync(join(fixtureDir, 'README.md'), '# Provider switch\n');
  const r = await ctx.post(`${base}/api/v1/workspaces`, { data: { name: 'Switch WS', root_path: fixtureDir } });
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

async function open(page: Page, provider: 'codex' | 'claude', title: string): Promise<{ id: string; thread: Locator }> {
  const r = await ctx.post(`${base}/api/v1/workspaces/${wsId}/runtime-sessions`, { data: { provider, cwd: fixtureDir, title } });
  expect(r.ok(), await r.text()).toBeTruthy();
  const id = (await r.json()).id as string;
  await page.goto(`/#/threads/${id}`);
  const thread = page.getByTestId(`runtime-thread-${id}`);
  await expect(thread).toBeVisible({ timeout: 30_000 });
  return { id, thread };
}

async function provider(id: string): Promise<string> {
  return ((await (await ctx.get(`${base}/api/v1/runtime-sessions/${id}`)).json()) as { provider: string }).provider;
}

test.beforeEach(async ({ page }, info) => {
  test.skip(info.project.name !== 'desktop-browser', 'desktop-browser project only');
  await page.addInitScript((id) => {
    localStorage.setItem('otto_workspace', id as string);
    localStorage.setItem('otto_theme', 'native');
  }, wsId);
  await page.setViewportSize({ width: 1440, height: 900 });
});

async function arm(page: Page, thread: Locator, to: string): Promise<void> {
  await thread.getByTestId('runtime-model').click();
  const picker = page.getByTestId('model-picker');
  await expectFullyInViewport(page, picker, 'model picker');
  await picker.getByTestId(`model-picker-provider-${to}`).click();
  await expect(picker.getByTestId('model-picker-switch-hint')).toBeVisible();
  // Browsing the rail arms nothing.
  await expect(thread.getByTestId('runtime-switch-notice')).toHaveCount(0);
  await picker.getByRole('button', { name: 'Default model' }).click();
  await page.keyboard.press('Escape');
  await expect(picker).toHaveCount(0);
}

test('claude → codex → claude: the same thread, a handoff divider each way', async ({ page }) => {
  test.setTimeout(150_000);
  const { id, thread } = await open(page, 'claude', 'Provider switch');
  const composer = thread.getByTestId('runtime-composer');
  await composer.fill('remember the word kiwi');
  await thread.getByTestId('runtime-send').click();
  await expect(thread.getByText('echo: remember the word kiwi').last()).toBeVisible({ timeout: 20_000 });

  // Arm, cancel, arm again: arming sends and stops nothing.
  await arm(page, thread, 'codex');
  const notice = thread.getByTestId('runtime-switch-notice');
  await expect(notice).toContainText('Next message continues on Codex');
  await expect(thread.getByTestId('runtime-model')).toHaveAttribute('aria-label', /Codex/);
  await notice.getByRole('button', { name: 'Cancel' }).click();
  await expect(notice).toHaveCount(0);
  // Back on the model Claude reported (the fake's own name).
  await expect(thread.getByTestId('runtime-model')).toHaveAttribute('aria-label', /fake-claude/);
  await arm(page, thread, 'codex');
  expect(await provider(id)).toBe('claude');
  for (const scheme of ['light', 'dark'] as const) {
    await page.emulateMedia({ colorScheme: scheme });
    if (SHOTS) await page.screenshot({ path: join(SHOTS, `desktop-armed-${scheme}.png`) });
  }
  await page.emulateMedia({ colorScheme: 'light' });

  await composer.fill('which word?');
  await thread.getByTestId('runtime-send').click();
  // Codex read the handoff, then the message whole.
  await expect(thread.getByText('Person: remember the word kiwi', { exact: false }).last()).toBeVisible({ timeout: 30_000 });
  const handoff = thread.getByTestId('runtime-handoff');
  await expect(handoff).toHaveCount(1);
  await expect(handoff.locator('summary')).toHaveText('Context handoff · Claude Code → Codex');
  await expect(notice).toHaveCount(0);
  await expect(thread.getByTestId('runtime-model')).not.toHaveAttribute('aria-label', /fake-claude/);
  expect(await provider(id)).toBe('codex');
  await handoff.locator('summary').click();
  await expect(handoff).toContainText('1 turn handed over.');
  await expect(handoff.locator('pre')).toContainText('[Context handoff from Claude Code]');
  await expectNoHorizontalOverflow(page);
  for (const scheme of ['light', 'dark'] as const) {
    await page.emulateMedia({ colorScheme: scheme });
    if (SHOTS) await page.screenshot({ path: join(SHOTS, `desktop-divider-${scheme}.png`) });
  }
  await page.emulateMedia({ colorScheme: 'light' });

  // And back: another handoff, a fresh Claude conversation.
  await arm(page, thread, 'claude');
  await composer.fill('and now?');
  await thread.getByTestId('runtime-send').click();
  await expect(thread.getByText('[Context handoff from Codex]', { exact: false }).last()).toBeVisible({ timeout: 30_000 });
  await expect(handoff).toHaveCount(2);
  await expect(handoff.nth(1).locator('summary')).toHaveText('Context handoff · Codex → Claude Code');
  expect(await provider(id)).toBe('claude');
  // Same visible thread throughout: one row in the sidebar.
  await expect(page.getByTestId(`runtime-thread-${id}`)).toBeVisible();
});
