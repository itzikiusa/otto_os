import { test, expect, type APIRequestContext, type Page } from '@playwright/test';
import { mkdtempSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { apiCtx } from './seed';

// A chat thread shown on Agents or Threads reads its own row and its linked
// group ONCE, then stays live on events: 5 idle seconds after it opens bring
// at most a handful of GET /runtime-sessions/{id} (+ /peers), in every way a
// thread pane is shown — a lone tab, split beside a terminal, tiled, restored
// from the saved layout, a closed (History) thread, a thread of another
// workspace, and the Threads page. 0.5.0 looped at ~41 reads/s on an Agents
// pane (the embed re-read its row whenever the pane's props object changed,
// and the read replaced the row the props were built from).

let ctx: APIRequestContext;
let base = '';
let wsA = '';
let wsB = '';
let dir = '';
let term = '';
const made: string[] = [];
const IDLE_MAX = 4;

async function thread(ws: string, title: string): Promise<string> {
  const t = await ctx.post(`${base}/api/v1/workspaces/${ws}/runtime-sessions`, { data: { provider: 'codex', cwd: dir, title } });
  expect(t.ok(), await t.text()).toBeTruthy();
  const id = (await t.json()).id as string;
  made.push(id);
  return id;
}

test.beforeAll(async () => {
  ({ ctx, base } = await apiCtx());
  dir = mkdtempSync(join(tmpdir(), 'otto-e2e-thread-reads-'));
  for (const name of ['Reads WS A', 'Reads WS B']) {
    const w = await ctx.post(`${base}/api/v1/workspaces`, { data: { name, root_path: dir } });
    expect(w.ok(), await w.text()).toBeTruthy();
    if (!wsA) wsA = (await w.json()).id as string;
    else wsB = (await w.json()).id as string;
  }
  const r = await ctx.post(`${base}/api/v1/workspaces/${wsA}/sessions`, {
    data: { kind: 'agent', provider: 'shell', title: 'Pirlo', cwd: dir, meta: { origin: 'e2e' } },
  });
  expect(r.ok(), await r.text()).toBeTruthy();
  term = (await r.json()).id as string;
});

test.afterAll(async () => {
  for (const id of made) await ctx.delete(`${base}/api/v1/runtime-sessions/${id}`).catch(() => {});
  if (term) await ctx.delete(`${base}/api/v1/sessions/${term}`).catch(() => {});
  await ctx?.dispose();
  if (dir) rmSync(dir, { recursive: true, force: true });
});

test.beforeEach(async ({ page }, info) => {
  test.skip(info.project.name !== 'desktop-browser', 'desktop-browser project only');
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.addInitScript((id) => {
    if (sessionStorage.getItem('thread-reads-init')) return;
    sessionStorage.setItem('thread-reads-init', '1');
    localStorage.setItem('otto_workspace', id as string);
    localStorage.setItem('otto_firstrun_dismissed', '1');
    localStorage.setItem('otto_nav_all_ws', '0');
    localStorage.setItem('otto_view_mode', 'tabs');
  }, wsA);
});

/** Every GET of `id`'s row or its peers, from now on. */
function readsOf(page: Page, id: string): string[] {
  const hits: string[] = [];
  page.on('request', (r) => {
    if (r.method() !== 'GET') return;
    const p = new URL(r.url()).pathname;
    if (p.endsWith(`/runtime-sessions/${id}`) || p.endsWith(`/runtime-sessions/${id}/peers`)) hits.push(p);
  });
  return hits;
}

/** The reads over `ms` idle milliseconds. */
async function idle(page: Page, hits: string[], label: string, ms = 5000): Promise<number> {
  const n = hits.length;
  await page.waitForTimeout(ms); // ui-guards: allow — counts what arrives in an idle window
  console.log(`[thread reads] ${label}: ${hits.length - n} in ${ms / 1000} idle s`);
  return hits.length - n;
}

const pane = (page: Page, id: string) => page.getByTestId(`thread-pane-${id}`);
/** The saved Agents tabs and pane layouts. */
const savedViews = (page: Page) =>
  page.evaluate(() => Object.keys(localStorage).filter((k) => /^otto_(tabs|panes)_/.test(k)).map((k) => localStorage.getItem(k)).join('\n'));

async function paneReady(page: Page, id: string): Promise<void> {
  await expect(pane(page, id)).toBeVisible({ timeout: 30_000 });
  await expect(pane(page, id).getByTestId(`runtime-thread-${id}`)).toBeVisible({ timeout: 30_000 });
  await page.waitForTimeout(500); // ui-guards: allow — let the open settle before the idle window
}

test('an Agents chat pane reads its thread once — a lone tab, split, tiled, restored', async ({ page }) => {
  const id = await thread(wsA, 'Nesta chat');
  const hits = readsOf(page, id);
  // A lone tab (a plain click on the row).
  await page.goto('/#/agents');
  await expect(page.getByTestId(`agents-row-${id}`)).toBeVisible({ timeout: 30_000 });
  await page.getByTestId(`agents-row-${id}`).click();
  await paneReady(page, id);
  expect(await idle(page, hits, 'tab'), 'reads in 5 idle s (tab)').toBeLessThanOrEqual(IDLE_MAX);

  // Split beside a terminal, then the same layout restored by a reload.
  await page.goto(`/#/agents/${term}`);
  await page.getByTestId(`agents-row-${id}`).click({ button: 'right' });
  await page.getByRole('menuitem', { name: 'Open beside' }).click();
  await paneReady(page, id);
  expect(await idle(page, hits, 'split'), 'reads in 5 idle s (split)').toBeLessThanOrEqual(IDLE_MAX);
  await page.reload();
  await paneReady(page, id);
  expect(await idle(page, hits, 'restored'), 'reads in 5 idle s (restored)').toBeLessThanOrEqual(IDLE_MAX);

  // Tiled.
  await page.evaluate(() => localStorage.setItem('otto_view_mode', 'tiled'));
  await page.reload();
  await paneReady(page, id);
  expect(await idle(page, hits, 'tiled'), 'reads in 5 idle s (tiled)').toBeLessThanOrEqual(IDLE_MAX);
  await page.evaluate(() => localStorage.setItem('otto_view_mode', 'tabs'));
});

test('a closed thread and another workspace’s thread read once too; so does Threads', async ({ page }) => {
  const closed = await thread(wsA, 'Gattuso chat');
  const r = await ctx.post(`${base}/api/v1/runtime-sessions/${closed}/close`);
  expect(r.ok(), await r.text()).toBeTruthy();
  const other = await thread(wsB, 'Seedorf chat');
  for (const id of [closed, other]) {
    const hits = readsOf(page, id);
    await page.goto(`/#/agents/${id}`);
    await paneReady(page, id);
    expect(await idle(page, hits, id === closed ? 'closed' : 'other workspace'), 'reads in 5 idle s').toBeLessThanOrEqual(IDLE_MAX);
  }
  const hits = readsOf(page, other);
  await page.goto(`/#/threads/${other}`);
  await expect(page.getByTestId(`runtime-thread-${other}`).first()).toBeVisible({ timeout: 30_000 });
  await page.waitForTimeout(500); // ui-guards: allow — let the open settle before the idle window
  expect(await idle(page, hits, 'Threads'), 'reads in 5 idle s (Threads)').toBeLessThanOrEqual(IDLE_MAX);
});

test('deleting a thread open as a pane reads it no more; the saved layout forgets its pane', async ({ page }) => {
  const id = await thread(wsA, 'Inzaghi chat');
  await page.goto(`/#/agents/${id}`);
  await paneReady(page, id);
  const hits = readsOf(page, id);
  let atDelete = -1;
  page.on('request', (r) => {
    if (r.method() === 'DELETE' && new URL(r.url()).pathname.endsWith(`/runtime-sessions/${id}`)) atDelete = hits.length;
  });
  await pane(page, id).getByRole('button', { name: /^More…/ }).click();
  await page.getByRole('menuitem', { name: 'Delete thread…' }).click();
  await page.getByRole('dialog').getByRole('button', { name: /^Delete/ }).click();
  await expect(pane(page, id)).toHaveCount(0, { timeout: 10_000 });
  await page.waitForTimeout(2000); // ui-guards: allow — proves an absence (no read after the delete)
  expect(atDelete, 'the thread was deleted').toBeGreaterThanOrEqual(0);
  expect(hits.slice(atDelete), 'reads after the delete').toEqual([]);
  // The saved layout forgets the pane.
  expect(await savedViews(page), 'the id left the saved tabs and panes').not.toContain(id);
});
