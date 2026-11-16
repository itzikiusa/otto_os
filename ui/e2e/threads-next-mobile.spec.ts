import { test, expect, type APIRequestContext, type Page } from '@playwright/test';
import { mkdtempSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { apiCtx } from './seed';
import { expectNoHorizontalOverflow } from './helpers';

// Real isolated daemon + fake provider processes, including real persisted drafts.
// These checks combine new features rather than replacing their focused protocol tests.
let ctx: APIRequestContext;
let base = '';
let workspace = '';
let root = '';
const sessions: string[] = [];

test.beforeAll(async () => {
  ({ ctx, base } = await apiCtx());
  root = mkdtempSync(join(tmpdir(), 'otto-threads-next-mobile-'));
  writeFileSync(join(root, 'README.md'), '# Threads phone acceptance\n');
  const response = await ctx.post(`${base}/api/v1/workspaces`, { data: { name: 'Threads continuity', root_path: root } });
  expect(response.ok(), await response.text()).toBeTruthy();
  workspace = (await response.json()).id as string;
});

test.afterAll(async () => {
  for (const id of sessions) await ctx.post(`${base}/api/v1/runtime-sessions/${id}/close`).catch(() => {});
  await ctx?.dispose();
  if (root) rmSync(root, { recursive: true, force: true });
});

async function openThread(page: Page, provider: 'claude' | 'codex', prompt: string): Promise<string> {
  const response = await ctx.post(`${base}/api/v1/workspaces/${workspace}/runtime-sessions`, {
    data: { provider, cwd: root, title: prompt },
  });
  expect(response.ok(), await response.text()).toBeTruthy();
  const id = (await response.json()).id as string;
  sessions.push(id);
  const sent = await ctx.post(`${base}/api/v1/runtime-sessions/${id}/prompt`, { data: { text: prompt } });
  expect(sent.ok(), await sent.text()).toBeTruthy();
  await page.addInitScript((ws) => localStorage.setItem('otto_workspace', ws), workspace);
  await page.goto(`/#/threads/${id}`);
  await expect(page.getByTestId(`runtime-thread-${id}`)).toBeVisible({ timeout: 30_000 });
  return id;
}

async function storedDraft(id: string): Promise<Record<string, unknown> | null> {
  const response = await ctx.get(`${base}/api/v1/runtime-sessions/${id}/draft`);
  expect(response.ok(), await response.text()).toBeTruthy();
  return (await response.json()).draft as Record<string, unknown> | null;
}

test('phone reload restores unsent text and attachment without sending', async ({ page }, info) => {
  const id = await openThread(page, 'codex', 'hello phone');
  const thread = page.getByTestId(`runtime-thread-${id}`);
  await expect(thread.getByText('echo: hello phone')).toBeVisible({ timeout: 15_000 });
  const composer = thread.getByTestId('runtime-composer');
  await composer.fill('Check this screenshot before changing anything');
  await thread.locator('input[type=file]').setInputFiles({
    name: 'draft.png', mimeType: 'image/png',
    buffer: Buffer.from('iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNk+M9QDwADhgGAWjR9awAAAABJRU5ErkJggg==', 'base64'),
  });
  await expect.poll(async () => (await storedDraft(id))?.text).toBe('Check this screenshot before changing anything');
  await expect.poll(async () => ((await storedDraft(id))?.images as unknown[])?.length).toBe(1);
  await page.reload();
  await expect(composer).toHaveValue('Check this screenshot before changing anything', { timeout: 30_000 });
  await expect(thread.getByTestId('runtime-images').getByRole('img', { name: 'draft.png' })).toBeVisible();
  await expect(thread.getByText('echo: Check this screenshot before changing anything')).toHaveCount(0);
  await expectNoHorizontalOverflow(page);
  await page.screenshot({ path: info.outputPath('draft-phone.png'), animations: 'disabled' });
});

test('phone clear intent survives reload and starts fresh inside the same visible thread', async ({ page }, info) => {
  const id = await openThread(page, 'claude', 'before fresh');
  const thread = page.getByTestId(`runtime-thread-${id}`);
  const composer = thread.getByTestId('runtime-composer');
  await expect(thread.getByText('echo: before fresh')).toBeVisible({ timeout: 15_000 });
  await composer.fill('/clear');
  await composer.press('Enter');
  await expect(thread.getByTestId('runtime-fresh-notice')).toContainText('Next message starts fresh');
  await composer.fill('after fresh');
  await expect.poll(async () => (await storedDraft(id))?.fresh_context).toBe(true);
  await expect.poll(async () => (await storedDraft(id))?.text).toBe('after fresh');
  await page.reload();
  await expect(composer).toHaveValue('after fresh', { timeout: 30_000 });
  await expect(thread.getByTestId('runtime-fresh-notice')).toBeVisible();
  const sent = page.waitForRequest((r) => r.method() === 'POST' && r.url().endsWith(`/runtime-sessions/${id}/prompt`));
  await thread.getByTestId('runtime-send').click();
  expect((await sent).postDataJSON().fresh_context).toBe(true);
  await expect(thread.getByText('echo: after fresh')).toBeVisible({ timeout: 15_000 });
  await expect(thread.getByText('echo: before fresh')).toBeVisible();
  await expect(thread.getByTestId('runtime-session-boundary')).toHaveCount(1);
  await expect(page).toHaveURL(new RegExp(`#/threads/${id}$`));
  await expect.poll(async () => (await storedDraft(id))?.fresh_context ?? false).toBe(false);
  await expectNoHorizontalOverflow(page);
  await page.screenshot({ path: info.outputPath('fresh-phone.png'), animations: 'disabled' });
});

test('phone tool images render and open a preview after conversation reload', async ({ page }, info) => {
  const id = await openThread(page, 'codex', 'tool image');
  const thread = page.getByTestId(`runtime-thread-${id}`);
  await expect(thread.locator('[data-item-type="dynamic_tool"]')).toHaveCount(1, { timeout: 15_000 });
  await page.reload();
  await expect(thread).toBeVisible({ timeout: 30_000 });
  for (const summary of await thread.locator('details:not([open]) > summary').all()) await summary.click();
  const tool = thread.locator('[data-item-type="dynamic_tool"]').first();
  await tool.locator('button.rw-line').click();
  await expect(tool.getByTestId('runtime-tool-images')).toBeVisible();
  await tool.getByRole('button', { name: 'Enlarge tool image 1', exact: true }).click();
  const preview = page.getByTestId('runtime-tool-image-enlarged');
  await expect(preview).toBeVisible();
  await expect.poll(() => preview.evaluate((el) => (el as HTMLImageElement).naturalWidth)).toBeGreaterThan(0);
  await expectNoHorizontalOverflow(page);
  await page.screenshot({ path: info.outputPath('tool-image-phone.png'), animations: 'disabled' });
  await page.getByRole('dialog').getByRole('button', { name: /close/i }).first().click();
  await expect(preview).toHaveCount(0);
});

test('phone recovered context can be inspected and removed without changing the draft', async ({ page }) => {
  const id = await openThread(page, 'codex', 'context phone');
  await expect(page.getByText('echo: context phone')).toBeVisible({ timeout: 15_000 });
  const existing = await (await ctx.get(`${base}/api/v1/runtime-sessions/${id}/draft`)).json();
  const saved = await ctx.put(`${base}/api/v1/runtime-sessions/${id}/draft`, { data: {
    revision: existing.revision,
    draft: { text: 'Is this review right?', context: [{ id: 'review-phone', kind: 'review-comment', label: 'Alex · src/check.ts:12', text: 'Alex: Handle an empty result.\nSam: Keep the current behavior for null.', author: 'Alex', path: 'src/check.ts', url: 'https://github.com/example/project/pull/1' }] },
  } });
  expect(saved.ok(), await saved.text()).toBeTruthy();
  await page.reload();
  const thread = page.getByTestId(`runtime-thread-${id}`);
  const composer = thread.getByTestId('runtime-composer');
  await expect(composer).toHaveValue('Is this review right?', { timeout: 30_000 });
  await thread.getByRole('button', { name: 'View context Alex · src/check.ts:12' }).click();
  const dialog = page.getByRole('dialog', { name: 'Message context' });
  await expect(dialog).toContainText('Sam: Keep the current behavior for null.');
  await expectNoHorizontalOverflow(page);
  await dialog.getByRole('button', { name: /close/i }).first().click();
  await thread.getByRole('button', { name: 'Remove Alex · src/check.ts:12' }).click();
  await expect(composer).toHaveValue('Is this review right?');
  await expect.poll(async () => ((await storedDraft(id))?.context as unknown[])?.length).toBe(0);
  await page.reload();
  await expect(composer).toHaveValue('Is this review right?', { timeout: 30_000 });
  await expect(thread.getByTestId('runtime-context-chips')).toHaveCount(0);
});

test('phone pending question survives reconnect and returns typed answers', async ({ page }, info) => {
  const id = await openThread(page, 'codex', 'question form');
  const thread = page.getByTestId(`runtime-thread-${id}`);
  await expect(thread.getByTestId('runtime-question-card')).toBeVisible({ timeout: 15_000 });
  await page.reload();
  const card = thread.getByTestId('runtime-question-card');
  await expect(card).toBeVisible({ timeout: 30_000 });
  await expect(thread.getByTestId('runtime-composer')).toHaveAttribute('placeholder', 'Answer the question above');
  await expect(thread.getByTestId('runtime-working')).toHaveCount(0);
  await card.getByLabel('Careful', { exact: false }).check();
  await card.getByLabel('Your answer').fill('Keep the public API unchanged');
  await expectNoHorizontalOverflow(page);
  await page.screenshot({ path: info.outputPath('question-phone-light.png'), animations: 'disabled' });
  await page.emulateMedia({ colorScheme: 'dark' });
  await page.screenshot({ path: info.outputPath('question-phone-dark.png'), animations: 'disabled' });
  await card.getByTestId('runtime-question-send').click();
  await expect(thread.getByText(/question answers:/)).toContainText('Keep the public API unchanged', { timeout: 15_000 });
  await expect(thread.getByText(/question answers:/)).toContainText('Careful');
  await expect(card.getByTestId('runtime-question-send')).toHaveCount(0);
  await expectNoHorizontalOverflow(page);
});
