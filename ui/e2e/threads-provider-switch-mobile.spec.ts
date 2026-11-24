import { test, expect, type APIRequestContext, type Locator, type Page } from '@playwright/test';
import { mkdtempSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { apiCtx } from './seed';
import { requireProviderSwitch } from './daemonFeatures';
import { expectFullyInViewport, expectNoHorizontalOverflow } from './helpers';

// ─────────────────────────────────────────────────────────────────────────────
// "Continue with another provider" on a phone (fake agents; taps only): the
// model sheet lists the other providers, a model there arms the switch, the
// armed line and the handoff divider fit a 390 px column, and the targets are
// ≥ 44 px. desktop-threads-provider-switch.spec.ts covers the full flow.
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
  await requireProviderSwitch(ctx, base, wsId, fixtureDir);
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
  test.skip(info.project.name !== 'iphone-portrait', 'iphone-portrait only');
  await page.addInitScript((id) => {
    localStorage.setItem('otto_workspace', id as string);
    localStorage.setItem('otto_theme', 'native');
  }, wsId);
});

test('codex → claude on a 390 px phone: arm in the sheet, the line and the divider fit', async ({ page }) => {
  test.setTimeout(150_000);
  await page.setViewportSize({ width: 390, height: 844 });
  const { id, thread } = await open(page, 'codex', 'Phone switch');
  const composer = thread.getByTestId('runtime-composer');
  await composer.fill('phone first');
  await thread.getByTestId('runtime-send').tap();
  await expect(thread.getByText('echo: phone first').last()).toBeVisible({ timeout: 20_000 });

  await thread.getByTestId('runtime-model').tap();
  const picker = page.getByTestId('model-picker');
  await expectFullyInViewport(page, picker, 'model picker');
  await picker.getByTestId('model-picker-provider-claude').tap();
  await expect(picker.getByTestId('model-picker-switch-hint')).toBeVisible();
  await picker.getByRole('button', { name: 'Default model' }).tap();
  await expect(picker).toHaveCount(0); // a pick that arms the switch closes the picker
  const notice = thread.getByTestId('runtime-switch-notice');
  await expect(notice).toContainText('Next message continues on Claude Code');
  await expectFullyInViewport(page, notice, 'armed line');
  await expectNoHorizontalOverflow(page);
  for (const scheme of ['light', 'dark'] as const) {
    await page.emulateMedia({ colorScheme: scheme });
    if (SHOTS) await page.screenshot({ path: join(SHOTS, `phone-armed-${scheme}.png`) });
  }
  await page.emulateMedia({ colorScheme: 'light' });

  await composer.fill('phone second');
  await thread.getByTestId('runtime-send').tap();
  await expect(thread.getByText('[Context handoff from Codex]', { exact: false }).last()).toBeVisible({ timeout: 30_000 });
  const summary = thread.getByTestId('runtime-handoff').locator('summary');
  await expect(summary).toHaveText('Context handoff · Codex → Claude Code');
  expect((await summary.boundingBox())!.height).toBeGreaterThanOrEqual(44);
  await summary.tap();
  await expect(thread.getByTestId('runtime-handoff')).toContainText('1 turn handed over.');
  await expectNoHorizontalOverflow(page);
  expect(await provider(id)).toBe('claude');
  await summary.scrollIntoViewIfNeeded();
  for (const scheme of ['light', 'dark'] as const) {
    await page.emulateMedia({ colorScheme: scheme });
    if (SHOTS) await page.screenshot({ path: join(SHOTS, `phone-divider-${scheme}.png`) });
  }
});
