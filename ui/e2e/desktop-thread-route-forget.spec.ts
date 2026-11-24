import { test, expect, type APIRequestContext, type Page } from '@playwright/test';
import { mkdtempSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { apiCtx } from './seed';

// A remembered route — the sidebar's resume point for Threads / Agents, the
// saved Agents tabs, a relaunch — whose thread was deleted since is dropped
// quietly: the route goes to the list and the id leaves the saved state. Only
// a link someone followed to a missing thread still says why ("Couldn't open
// this thread"). 0.5.0 toasted that on every visit to Threads.

let ctx: APIRequestContext;
let base = '';
let wsId = '';
let dir = '';
const made: string[] = [];

async function thread(title: string): Promise<string> {
  const t = await ctx.post(`${base}/api/v1/workspaces/${wsId}/runtime-sessions`, { data: { provider: 'codex', cwd: dir, title } });
  expect(t.ok(), await t.text()).toBeTruthy();
  const id = (await t.json()).id as string;
  made.push(id);
  return id;
}

async function remove(id: string): Promise<void> {
  const d = await ctx.delete(`${base}/api/v1/runtime-sessions/${id}`);
  expect(d.ok(), await d.text()).toBeTruthy();
}

test.beforeAll(async () => {
  ({ ctx, base } = await apiCtx());
  dir = mkdtempSync(join(tmpdir(), 'otto-e2e-thread-forget-'));
  const w = await ctx.post(`${base}/api/v1/workspaces`, { data: { name: 'Forget WS', root_path: dir } });
  expect(w.ok(), await w.text()).toBeTruthy();
  wsId = (await w.json()).id as string;
});

test.afterAll(async () => {
  for (const id of made) await ctx.delete(`${base}/api/v1/runtime-sessions/${id}`).catch(() => {});
  await ctx?.dispose();
  if (dir) rmSync(dir, { recursive: true, force: true });
});

test.beforeEach(async ({ page }, info) => {
  test.skip(info.project.name !== 'desktop-browser', 'desktop-browser project only');
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.addInitScript((id) => {
    if (sessionStorage.getItem('thread-forget-init')) return;
    sessionStorage.setItem('thread-forget-init', '1');
    localStorage.setItem('otto_workspace', id as string);
    localStorage.setItem('otto_firstrun_dismissed', '1');
    localStorage.setItem('otto_nav_all_ws', '0');
    localStorage.setItem('otto_view_mode', 'tabs');
  }, wsId);
});

const hash = (page: Page) => page.evaluate(() => location.hash);
const navTo = (page: Page, id: string) => page.locator(`:is(.nav-item, .rail-btn)[data-nav-id="${id}"]:visible`).first().click();
const openToast = (page: Page) => page.getByText(/Couldn’t open this (thread|session)/);
/** Where routes, Threads panes and Agents tabs/panes are remembered. */
const memory = (page: Page) =>
  page.evaluate(() =>
    Object.keys(localStorage)
      .filter((k) => /^otto_(last_by_module|last_route|threads_v1|tabs_|panes_)/.test(k))
      .map((k) => localStorage.getItem(k))
      .join('\n'),
  );

async function quietly(page: Page): Promise<void> {
  await page.waitForTimeout(1500); // ui-guards: allow — proves an absence (no toast)
  await expect(openToast(page)).toHaveCount(0);
}

test('Threads: a remembered route to a deleted thread goes to the list quietly; a followed link still says why', async ({ page }) => {
  const mine = await thread('Pirlo chat');
  const later = await thread('Rui Costa chat');
  // Deleted in the app (from its Agents pane); the sidebar resumes Threads on it.
  await page.goto(`/#/threads/${mine}`);
  await expect(page.getByTestId(`runtime-thread-${mine}`).first()).toBeVisible({ timeout: 30_000 });
  await page.goto(`/#/agents/${mine}`);
  const pane = page.getByTestId(`thread-pane-${mine}`);
  await expect(pane).toBeVisible({ timeout: 30_000 });
  await pane.getByRole('button', { name: /^More…/ }).click();
  await page.getByRole('menuitem', { name: 'Delete thread…' }).click();
  await page.getByRole('dialog').getByRole('button', { name: /^Delete/ }).click();
  await expect(pane).toHaveCount(0, { timeout: 10_000 });
  await navTo(page, 'threads');
  await quietly(page);
  await expect.poll(() => hash(page)).toBe('#/threads');
  expect(await memory(page)).not.toContain(mine);

  // Deleted elsewhere (the API), then a relaunch with the stale id remembered.
  await page.goto(`/#/threads/${later}`);
  await expect(page.getByTestId(`runtime-thread-${later}`).first()).toBeVisible({ timeout: 30_000 });
  await navTo(page, 'agents');
  await expect.poll(() => hash(page)).toMatch(/^#\/agents/);
  await remove(later);
  await page.reload();
  await navTo(page, 'threads');
  await quietly(page);
  await expect.poll(() => hash(page)).toBe('#/threads');
  expect(await memory(page)).not.toContain(later);

  // Someone followed a link to it: say why.
  await page.goto(`/#/threads/${mine}`);
  await expect(page.getByText('Couldn’t open this thread')).toBeVisible({ timeout: 10_000 });
});

test('Agents: a remembered route or saved pane of a deleted thread is dropped quietly', async ({ page }) => {
  const gone = await thread('Kaka chat');
  await page.goto(`/#/agents/${gone}`);
  await expect(page.getByTestId(`thread-pane-${gone}`)).toBeVisible({ timeout: 30_000 });
  await navTo(page, 'threads');
  await expect.poll(() => hash(page)).toMatch(/^#\/threads/);
  await remove(gone);
  // A relaunch: the saved tab and the resume point both still name it.
  await page.reload();
  await navTo(page, 'agents');
  await quietly(page);
  await expect.poll(() => hash(page)).toBe('#/agents');
  await expect(page.getByTestId(`thread-pane-${gone}`)).toHaveCount(0);
  expect(await memory(page)).not.toContain(gone);

  // Only the route remembers it (its tab was closed in another window).
  const also = await thread('Crespo chat');
  await page.goto(`/#/agents/${also}`);
  await expect(page.getByTestId(`thread-pane-${also}`)).toBeVisible({ timeout: 30_000 });
  await navTo(page, 'threads');
  await expect.poll(() => hash(page)).toMatch(/^#\/threads/);
  await remove(also);
  await page.evaluate(() => Object.keys(localStorage).filter((k) => /^otto_(tabs|panes)_/.test(k)).forEach((k) => localStorage.removeItem(k)));
  await page.reload();
  await navTo(page, 'agents');
  await quietly(page);
  await expect.poll(() => hash(page)).toBe('#/agents');
  expect(await memory(page)).not.toContain(also);
  // Someone followed a link to it: say why.
  await page.goto(`/#/agents/${gone}`);
  await expect(page.getByText('Couldn’t open this session')).toBeVisible({ timeout: 10_000 });
});

test('deleting the thread open on Threads reads it no more and leaves nothing behind', async ({ page }) => {
  const id = await thread('Ambrosini chat');
  await page.goto(`/#/threads/${id}`);
  const view = page.getByTestId(`runtime-thread-${id}`).first();
  await expect(view).toBeVisible({ timeout: 30_000 });
  const after: string[] = [];
  let deleted = false;
  page.on('request', (r) => {
    const p = new URL(r.url()).pathname;
    if (!p.includes(id)) return;
    if (r.method() === 'DELETE') deleted = true;
    else if (deleted) after.push(`${r.method()} ${p}`);
  });
  await page.getByTestId('threads-thread-menu').click();
  await page.locator('.ctx-menu').getByRole('menuitem', { name: 'Delete thread…' }).click();
  await page.getByRole('dialog').getByRole('button', { name: /^Delete/ }).click();
  await expect(view).toHaveCount(0, { timeout: 10_000 });
  await quietly(page);
  expect(deleted, 'the thread was deleted').toBe(true);
  expect(after, 'reads after the delete').toEqual([]);
  expect(await memory(page)).not.toContain(id);
});
