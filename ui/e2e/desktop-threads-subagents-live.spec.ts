import { test, expect, type APIRequestContext } from '@playwright/test';
import { mkdtempSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { apiCtx } from './seed';

// ─────────────────────────────────────────────────────────────────────────────
// A person delegates from the composer (`@cod` → Codex) on the real daemon
// with the fake Codex, which answers at once. Once the subagent's turn ended
// the thread must not keep saying "Waiting on 1 subagent" (review B-1: the
// ask's HTTP answer, carrying the row as it was when the turn started, landed
// after the live "idle" event and overwrote it until a reload).
// ─────────────────────────────────────────────────────────────────────────────

let ctx: APIRequestContext;
let base = '';
let wsId = '';
let dir = '';

test.beforeAll(async () => {
  const c = await apiCtx();
  ctx = c.ctx;
  base = c.base;
  dir = mkdtempSync(join(tmpdir(), 'otto-e2e-sub-live-'));
  writeFileSync(join(dir, 'README.md'), '# Subagents live\n');
  const r = await ctx.post(`${base}/api/v1/workspaces`, { data: { name: 'Subagents live WS', root_path: dir } });
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
    if (sessionStorage.getItem('sub-live-init')) return;
    sessionStorage.setItem('sub-live-init', '1');
    localStorage.setItem('otto_workspace', id as string);
    localStorage.removeItem('otto_threads_v1');
  }, wsId);
});

test('a person-delegated subagent that finished settles without a reload', async ({ page }) => {
  const created = await ctx.post(`${base}/api/v1/workspaces/${wsId}/runtime-sessions`, { data: { provider: 'claude', cwd: dir, title: 'Delegate live' } });
  expect(created.ok(), await created.text()).toBeTruthy();
  const id = (await created.json()).id as string;
  await page.goto(`/#/threads/${id}`);
  const thread = page.getByTestId(`runtime-thread-${id}`);
  await expect(thread).toBeVisible({ timeout: 30_000 });
  const composer = thread.getByTestId('runtime-composer');
  await composer.fill('Write a one line summary @cod');
  await composer.press('Enter');
  await composer.pressSequentially('@gem');
  await composer.press('Enter');
  await expect(thread.getByTestId('runtime-ask-targets')).toContainText('Codex');
  await expect(thread.getByTestId('runtime-ask-targets')).toContainText('Gemini');
  await composer.press('Enter');
  // The subagent answered (the API says so)…
  await expect
    .poll(async () => {
      const rows = (await (await ctx.get(`${base}/api/v1/workspaces/${wsId}/runtime-sessions`)).json()) as { id: string; status: string; last_turn_status?: string; meta?: { child_of?: { id: string } } }[];
      return rows.filter((r) => r.meta?.child_of?.id === id).map((r) => [r.status, r.last_turn_status]);
    }, { timeout: 30_000 })
    .toEqual([['idle', 'completed'], ['idle', 'completed']]);
  // The parent's agent was woken with the replies; let it finish too.
  await expect
    .poll(async () => ((await (await ctx.get(`${base}/api/v1/runtime-sessions/${id}`)).json()) as { status: string }).status, { timeout: 30_000 })
    .toBe('idle');
  // …so the thread says so too: no waiting line, the row reads Done.
  await expect(thread.getByTestId('runtime-bg-waiting')).toHaveCount(0, { timeout: 5_000 });
  const group = thread.getByTestId('runtime-child-group-summary');
  await expect(group).toHaveText('2 done');
});
