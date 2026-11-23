import { test, expect, type APIRequestContext } from '@playwright/test';
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { apiCtx } from './seed';
import { expectFullyInViewport, expectNoHorizontalOverflow } from './helpers';

// A council on a phone, with REAL fake-agent seats: one seat at a time, and
// the council strip is a seat switcher — finger-sized chips that switch seats
// in one tap — with every council action in one "Council" menu.

let ctx: APIRequestContext;
let base = '';
let wsId = '';
let dir = '';

test.beforeAll(async ({}, info) => {
  if (!/^(iphone|android)/.test(info.project.name)) return;
  const c = await apiCtx();
  ctx = c.ctx;
  base = c.base;
  dir = mkdtempSync(join(tmpdir(), 'otto-e2e-council-phone-'));
  writeFileSync(join(dir, 'README.md'), '# Council phone\n');
  const r = await ctx.post(`${base}/api/v1/workspaces`, { data: { name: 'Council phone WS', root_path: dir } });
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

test('the council switcher changes seat in one tap and keeps every action in one menu', async ({ page }, info) => {
  test.skip(!/^(iphone|android)/.test(info.project.name) || info.project.name.includes('landscape'), 'portrait phones only');
  test.setTimeout(120_000);
  await page.addInitScript((id) => {
    if (sessionStorage.getItem('council-phone-init')) return;
    sessionStorage.setItem('council-phone-init', '1');
    localStorage.setItem('otto_workspace', id as string);
    localStorage.removeItem('otto_threads_v1');
  }, wsId);
  const r = await ctx.post(`${base}/api/v1/workspaces/${wsId}/runtime-sessions/council`, {
    data: { prompt: 'Pick a queue', title: 'Queue choice', cwd: dir, seats: [{ provider: 'claude' }, { provider: 'codex' }] },
  });
  expect(r.ok(), await r.text()).toBeTruthy();
  const [s1, s2] = ((await r.json()).seats as { id: string }[]).map((s) => s.id);

  await page.goto(`/#/threads/${s1}`);
  await expect(page.getByTestId(`runtime-thread-${s1}`)).toBeVisible({ timeout: 30_000 });
  const strip = page.getByTestId('threads-pane-strip');
  await expect(strip).toBeVisible();
  const chip2 = strip.getByTestId(`pane-chip-${s2}`);
  await expect(strip.getByTestId(`pane-chip-${s1}`)).toHaveAttribute('aria-pressed', 'true');
  for (const id of [s1, s2]) {
    const box = (await strip.getByTestId(`pane-chip-${id}`).boundingBox())!;
    expect(box.height).toBeGreaterThanOrEqual(43.9);
  }
  await expectNoHorizontalOverflow(page);
  const out = process.env.OTTO_E2E_SHOTS;
  const shot = async (name: string): Promise<void> => {
    if (!out) return;
    mkdirSync(join(out, 'fix-T-COUNCIL'), { recursive: true });
    for (const scheme of ['light', 'dark'] as const) {
      await page.emulateMedia({ colorScheme: scheme });
      await page.screenshot({ path: join(out, 'fix-T-COUNCIL', `${info.project.name}-${name}-${scheme}.png`), animations: 'disabled' });
    }
    await page.emulateMedia({ colorScheme: 'light' });
  };
  await shot('council-switcher');

  // One tap: seat 2 as the full thread.
  await chip2.click();
  await expect(page.getByTestId(`runtime-thread-${s2}`)).toBeVisible({ timeout: 15_000 });
  await expect(chip2).toHaveAttribute('aria-pressed', 'true');

  // Every council action sits in one menu that fits the phone.
  await strip.getByTestId('pane-strip-council-menu').click();
  const menu = page.locator('.ctx-menu').first();
  await expect(menu.getByRole('menuitem', { name: 'Ask every model…' })).toBeVisible();
  await expect(menu.getByRole('menuitem', { name: /Compare answers with/ })).toBeVisible();
  await expect(menu.getByRole('menuitem', { name: 'Continue with this answer' })).toBeVisible();
  await expectFullyInViewport(page, menu, 'Council menu (phone)');
  await shot('council-menu');
  await page.keyboard.press('Escape');
  await expectNoHorizontalOverflow(page);
});
