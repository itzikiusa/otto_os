import { test, expect, type APIRequestContext } from '@playwright/test';
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { apiCtx } from './seed';
import { expectFullyInViewport, expectNoHorizontalOverflow } from './helpers';

// ─────────────────────────────────────────────────────────────────────────────
// Thread details' "Agents & tasks" opens what it lists, and image previews are
// readable. A Claude thread (otto-fake-claude via global-setup's claude shim):
// `bg 3` starts a background command whose output Claude writes to a file the
// daemon reads back; `agent 3` a background subagent with its own activity.
// ─────────────────────────────────────────────────────────────────────────────

let ctx: APIRequestContext;
let base = '';
let wsId = '';
let fixtureDir = '';

test.beforeAll(async () => {
  const c = await apiCtx();
  ctx = c.ctx;
  base = c.base;
  fixtureDir = mkdtempSync(join(tmpdir(), 'otto-e2e-agents-tasks-'));
  writeFileSync(join(fixtureDir, 'README.md'), '# Agents and tasks\n');
  const r = await ctx.post(`${base}/api/v1/workspaces`, { data: { name: 'Agents Tasks WS', root_path: fixtureDir } });
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
    if (sessionStorage.getItem('agents-tasks-init')) return;
    sessionStorage.setItem('agents-tasks-init', '1');
    localStorage.setItem('otto_workspace', id as string);
    localStorage.removeItem('otto_threads_v1');
  }, wsId);
});

async function claudeThread(title: string): Promise<string> {
  const created = await ctx.post(`${base}/api/v1/workspaces/${wsId}/runtime-sessions`, { data: { provider: 'claude', cwd: fixtureDir, title } });
  expect(created.ok(), await created.text()).toBeTruthy();
  return (await created.json()).id as string;
}

test('Agents & tasks rows open the subagent and the background command, desktop and phone', async ({ page }, info) => {
  const id = await claudeThread('Agents and tasks');
  await page.goto(`/#/threads/${id}`);
  const thread = page.getByTestId(`runtime-thread-${id}`);
  await expect(thread).toBeVisible({ timeout: 30_000 });
  const composer = thread.getByTestId('runtime-composer');

  // A background command: the strip opens Agents & tasks, the row its output.
  await composer.fill('bg 3');
  await composer.press('Enter');
  await expect(thread.getByText('Waiting for it.')).toBeVisible({ timeout: 15_000 });
  await thread.getByTestId('runtime-background').click();
  const panel = page.getByTestId('thread-agents-tasks');
  await expect(panel).toBeVisible();
  const cmdRow = panel.getByTestId('agents-tasks-row').filter({ hasText: 'sleep 300' });
  await cmdRow.getByRole('button').click();
  const detail = panel.getByTestId('agents-tasks-detail');
  await expect(detail.getByTestId('runtime-task-output')).toContainText('tick 2', { timeout: 10_000 });
  await expect(detail).toContainText('sleep 300');
  await expectNoHorizontalOverflow(page);
  for (const scheme of ['light', 'dark'] as const) {
    await page.emulateMedia({ colorScheme: scheme });
    await page.screenshot({ path: info.outputPath(`agents-tasks-command-${scheme}.png`), animations: 'disabled' });
  }
  await page.emulateMedia({ colorScheme: 'light' });
  await panel.getByTestId('agents-tasks-back').click();
  await expect(panel.getByTestId('agents-tasks-detail')).toHaveCount(0);
  await expect(thread.getByText('Background done: ok')).toBeVisible({ timeout: 20_000 });

  // A background subagent: the same card as the timeline, opened in place.
  await composer.fill('agent 3');
  await composer.press('Enter');
  await expect(thread.getByText("I'm waiting for the PR sweep to come back.")).toBeVisible({ timeout: 15_000 });
  const agentRow = panel.getByTestId('agents-tasks-row').filter({ hasText: 'PR sweep' });
  await expect(agentRow).toContainText('Background');
  await expect(agentRow).toContainText('Running');
  await agentRow.getByRole('button', { name: /PR sweep/ }).click();
  await expect(detail).toContainText('Sweep the open PRs');
  await expect(detail).toContainText('Looking at PRs');
  for (const scheme of ['light', 'dark'] as const) {
    await page.emulateMedia({ colorScheme: scheme });
    await page.screenshot({ path: info.outputPath(`agents-tasks-agent-${scheme}.png`), animations: 'disabled' });
  }
  await page.emulateMedia({ colorScheme: 'light' });
  await expect(detail).toContainText('Found 2 open PRs', { timeout: 20_000 });
  await panel.getByTestId('agents-tasks-back').click();

  // Phone width: the strip opens the drawer, its row a sheet that fits.
  await page.setViewportSize({ width: 390, height: 844 });
  await composer.fill('agent 4');
  await composer.press('Enter');
  await thread.getByTestId('runtime-background').click({ timeout: 15_000 });
  const phonePanel = page.getByTestId('thread-agents-tasks');
  await expect(phonePanel).toBeVisible();
  await phonePanel.getByTestId('agents-tasks-row').filter({ hasText: 'Running' }).getByRole('button', { name: /PR sweep/ }).click();
  const sheet = page.getByRole('dialog', { name: 'PR sweep', exact: true });
  await expect(sheet).toContainText('Sweep the open PRs');
  await expectFullyInViewport(page, sheet);
  await expectNoHorizontalOverflow(page);
  for (const scheme of ['light', 'dark'] as const) {
    await page.emulateMedia({ colorScheme: scheme });
    await page.screenshot({ path: info.outputPath(`agents-tasks-phone-${scheme}.png`), animations: 'disabled' });
  }
  await page.emulateMedia({ colorScheme: 'light' });
  await page.keyboard.press('Escape');
  await expect(sheet).toHaveCount(0);
});

test('image previews are readable and open full size', async ({ page }, info) => {
  const id = await claudeThread('Image previews');
  await page.goto(`/#/threads/${id}`);
  const thread = page.getByTestId(`runtime-thread-${id}`);
  await expect(thread).toBeVisible({ timeout: 30_000 });
  const png = readFileSync(new URL('../../docs/testing/telemetry-images/telemetry-phone.png', import.meta.url));
  await thread.locator('input[type=file]').setInputFiles({ name: 'shot.png', mimeType: 'image/png', buffer: png });
  const preview = thread.getByTestId('runtime-images').getByRole('img', { name: 'shot.png' });
  await expect(preview).toBeVisible();
  const box = (await preview.boundingBox())!;
  expect(Math.max(box.width, box.height)).toBeGreaterThanOrEqual(150);
  await thread.getByTestId('runtime-images').getByRole('button', { name: 'Open shot.png' }).click();
  const full = page.getByTestId('runtime-image-full');
  await expect(full).toBeVisible();
  await expectFullyInViewport(page, full);
  expect((await full.boundingBox())!.height).toBeGreaterThan(box.height);
  for (const scheme of ['light', 'dark'] as const) {
    await page.emulateMedia({ colorScheme: scheme });
    await page.screenshot({ path: info.outputPath(`image-full-${scheme}.png`), animations: 'disabled' });
  }
  await page.emulateMedia({ colorScheme: 'light' });
  await page.keyboard.press('Escape');
  await expect(full).toHaveCount(0);
  // × still removes it.
  await expect(thread.getByTestId('runtime-images').getByRole('button', { name: 'Remove shot.png' })).toBeVisible();

  // Sent: the message's thumbnail is as readable and opens full size too.
  const composer = thread.getByTestId('runtime-composer');
  await composer.fill('look');
  await composer.press('Enter');
  const sentThumb = thread.getByTestId('runtime-attachments').getByRole('button', { name: /^Open / }).first();
  await expect(sentThumb.locator('img')).toBeVisible({ timeout: 15_000 });
  const sentBox = (await sentThumb.locator('img').boundingBox())!;
  expect(Math.max(sentBox.width, sentBox.height)).toBeGreaterThanOrEqual(150);
  await sentThumb.click();
  await expect(page.getByTestId('runtime-attachment-full')).toBeVisible();
  await page.keyboard.press('Escape');

  // Phone: previews fit without horizontal scroll.
  await page.setViewportSize({ width: 390, height: 844 });
  await expect(sentThumb.locator('img')).toBeVisible();
  await expectNoHorizontalOverflow(page);
  await page.screenshot({ path: info.outputPath('image-previews-phone.png'), animations: 'disabled' });
});
