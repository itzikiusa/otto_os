import { test, expect, type APIRequestContext, type Page } from '@playwright/test';
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { apiCtx } from './seed';
import { expectFullyInViewport, expectNoHorizontalOverflow } from './helpers';

// The design review's phone findings, by touch on every phone and tablet:
//   • the model picker is a titled bottom sheet ("Model") on a phone, names
//     every provider, and closes once a pick arms a switch;
//   • the "Next message continues on …" line has a 44 px Cancel and nothing
//     inside it scrolls (no stray scrollbars);
//   • the top bar keeps the thread's ⋯ and right-panel buttons and ends a
//     long title with an ellipsis.
// Screenshots: OTTO_E2E_SHOTS/review-round/.

test.use({ serviceWorkers: 'block' });
const MOBILE = /^(iphone-|ipad-|android-)/;

let ctx: APIRequestContext;
let base = '';
let wsId = '';
let dir = '';

test.beforeAll(async ({}, info) => {
  if (!MOBILE.test(info.project.name)) return;
  const c = await apiCtx();
  ctx = c.ctx;
  base = c.base;
  dir = mkdtempSync(join(tmpdir(), 'otto-e2e-review-round-m-'));
  writeFileSync(join(dir, 'README.md'), '# review round\n');
  const r = await ctx.post(`${base}/api/v1/workspaces`, { data: { name: 'Review round phone WS', root_path: dir } });
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
  test.skip(!MOBILE.test(info.project.name), 'phones and tablets');
  await page.addInitScript((id) => localStorage.setItem('otto_workspace', id as string), wsId);
});

async function shot(page: Page, name: string, project: string): Promise<void> {
  const out = process.env.OTTO_E2E_SHOTS;
  if (!out) return;
  mkdirSync(join(out, 'review-round'), { recursive: true });
  for (const scheme of ['light', 'dark'] as const) {
    await page.emulateMedia({ colorScheme: scheme });
    await page.screenshot({ path: join(out, 'review-round', `${project}-${name}-${scheme}.png`), animations: 'disabled' });
  }
  await page.emulateMedia({ colorScheme: 'light' });
}

test('touch: model picker (a "Model" sheet on a phone), the armed switch line, the top bar', async ({ page }, info) => {
  const title = 'Rewrite the quickstart guide for people who have never used the command line';
  const r = await ctx.post(`${base}/api/v1/workspaces/${wsId}/runtime-sessions`, { data: { provider: 'codex', cwd: dir, title } });
  const id = (await r.json()).id as string;
  await page.goto(`/#/threads/${id}`);
  const thread = page.getByTestId(`runtime-thread-${id}`);
  await expect(thread).toBeVisible({ timeout: 30_000 });
  // The top bar: its own ⋯ and the right panel stay; the title ends with an ellipsis.
  await expect(page.getByTestId('threads-thread-menu')).toBeVisible();
  await expect(page.getByTestId('threads-toggle-inspector')).toBeVisible();
  const crumb = page.locator('.th-crumb-title').first();
  expect(await crumb.evaluate((el) => getComputedStyle(el).textOverflow)).toBe('ellipsis');
  expect(await crumb.evaluate((el) => el.getBoundingClientRect().right)).toBeLessThanOrEqual((await page.getByTestId('threads-toggle-inspector').boundingBox())!.x + 1);

  await thread.getByTestId('runtime-model').tap();
  const picker = page.getByTestId('model-picker');
  await expect(picker).toBeVisible();
  await expect(picker.getByTestId('model-picker-provider-codex')).toContainText('Codex');
  const phone = (page.viewportSize()?.width ?? 1000) <= 640;
  if (phone) {
    await expect(picker.getByRole('heading', { name: 'Model' })).toBeVisible();
    const box = (await picker.boundingBox())!;
    expect(Math.round(box.y + box.height), 'a bottom sheet').toBeGreaterThanOrEqual(page.viewportSize()!.height - 2);
  }
  await expectFullyInViewport(page, picker, 'model picker');
  await shot(page, 'model-picker', info.project.name);
  await picker.getByTestId('model-picker-provider-claude').tap();
  await picker.locator('[data-mp-idx="0"]').tap();
  await expect(picker).toHaveCount(0);

  const notice = thread.getByTestId('runtime-switch-notice');
  await expect(notice).toContainText('Next message continues on');
  const cancel = notice.getByRole('button', { name: 'Cancel' });
  expect((await cancel.boundingBox())!.height, 'Cancel is a 44 px target').toBeGreaterThanOrEqual(44);
  const scrollers = await notice.evaluate((el) => {
    const out: string[] = [];
    for (let n: HTMLElement | null = el as HTMLElement; n && !n.classList.contains('rt'); n = n.parentElement) {
      if (n.scrollWidth > n.clientWidth + 1 || n.scrollHeight > n.clientHeight + 1) out.push(`${n.className} ${n.scrollWidth}x${n.scrollHeight} > ${n.clientWidth}x${n.clientHeight}`);
    }
    return out;
  });
  expect(scrollers, 'nothing in the notice scrolls').toEqual([]);
  await expectNoHorizontalOverflow(page);
  await shot(page, 'switch-armed', info.project.name);
  await cancel.tap();
  await expect(notice).toHaveCount(0);
});
