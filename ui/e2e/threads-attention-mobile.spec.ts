import { test, expect, type APIRequestContext, type Page } from '@playwright/test';
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { apiCtx } from './seed';
import { expectFullyInViewport, expectNoHorizontalOverflow } from './helpers';

// ─────────────────────────────────────────────────────────────────────────────
// The attention dock on a phone (portrait and landscape), with REAL fake-agent
// turns (otto-fake-codex `ask` / `question form`): what the agent waits for
// sits above the composer with its answers in view and on top — one Approve,
// one Send answer — at 44 px, with no horizontal scroll. Screenshots in
// OTTO_E2E_SHOTS/fix-T-THREAD/.
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
  fixtureDir = mkdtempSync(join(tmpdir(), 'otto-e2e-threads-attn-m-'));
  writeFileSync(join(fixtureDir, 'README.md'), '# Attention on a phone\n');
  const r = await ctx.post(`${base}/api/v1/workspaces`, { data: { name: 'Attention Phone WS', root_path: fixtureDir } });
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
  test.skip(!/^iphone-(portrait|landscape)$/.test(info.project.name), 'iPhone portrait and landscape');
  await page.addInitScript((id) => {
    localStorage.setItem('otto_workspace', id as string);
  }, wsId);
});

async function shot(page: Page, name: string, project: string): Promise<void> {
  const dir = process.env.OTTO_E2E_SHOTS;
  if (!dir) return;
  mkdirSync(join(dir, 'fix-T-THREAD'), { recursive: true });
  await page.screenshot({ path: join(dir, 'fix-T-THREAD', `${project}-${name}.png`), animations: 'disabled' });
}

async function onTop(loc: import('@playwright/test').Locator): Promise<boolean> {
  const box = (await loc.boundingBox())!;
  return loc.evaluate((el, p) => {
    const hit = document.elementFromPoint(p.x, p.y);
    return !!hit && (hit === el || el.contains(hit));
  }, { x: box.x + box.width / 2, y: box.y + box.height / 2 });
}

async function openThread(page: Page, title: string, prompt: string) {
  const created = await ctx.post(`${base}/api/v1/workspaces/${wsId}/runtime-sessions`, {
    data: { provider: 'codex', cwd: fixtureDir, title },
  });
  expect(created.ok(), await created.text()).toBeTruthy();
  const id = (await created.json()).id as string;
  await page.goto(`/#/threads/${id}`);
  const thread = page.getByTestId(`runtime-thread-${id}`);
  await expect(thread).toBeVisible({ timeout: 30_000 });
  const composer = thread.getByTestId('runtime-composer');
  await composer.fill(prompt);
  await thread.getByTestId('runtime-send').click();
  return { id, thread, composer };
}

test('phone: an approval waits in the dock with its answers in view', async ({ page }, info) => {
  const { thread } = await openThread(page, 'Phone approval', 'ask');
  const dock = thread.getByTestId('runtime-attention');
  await expect(dock).toHaveAttribute('data-kind', 'approval', { timeout: 20_000 });
  const approve = dock.getByRole('button', { name: 'Approve', exact: true });
  await expect(thread.getByRole('button', { name: 'Approve', exact: true })).toHaveCount(1);
  for (const name of ['Approve', 'Decline']) {
    const btn = dock.getByRole('button', { name, exact: true });
    await expectFullyInViewport(page, btn, `${name} in the dock`);
    expect((await btn.boundingBox())!.height, `${name} tap target`).toBeGreaterThanOrEqual(43.9);
    expect(await onTop(btn), `${name} is on top`).toBe(true);
  }
  // No desktop key hints on touch.
  await expect(dock.locator('kbd')).toBeHidden();
  await expectFullyInViewport(page, thread.getByTestId('runtime-composer'), 'composer under the dock');
  await expectNoHorizontalOverflow(page);
  await shot(page, 'approval', info.project.name);
  await approve.click();
  await expect(dock).toHaveCount(0, { timeout: 20_000 });
});

test('phone: a question form answers in the dock with one Send in view', async ({ page }, info) => {
  const { id, thread } = await openThread(page, 'Phone question', 'question form');
  const dock = thread.getByTestId('runtime-attention');
  await expect(dock).toHaveAttribute('data-kind', 'question', { timeout: 20_000 });
  const send = thread.getByTestId('runtime-question-send');
  await expect(send).toHaveCount(1);
  await expectFullyInViewport(page, send, 'Send answer');
  expect(await onTop(send), 'Send answer is on top').toBe(true);
  expect((await send.boundingBox())!.height).toBeGreaterThanOrEqual(43.9);
  // The fields scroll inside the dock; Send stays put while they do.
  await dock.getByLabel('Careful', { exact: false }).check();
  const answer = dock.getByLabel('Your answer', { exact: true });
  await answer.scrollIntoViewIfNeeded();
  await answer.fill('Keep the API stable');
  await expectFullyInViewport(page, send, 'Send answer after typing');
  await expectNoHorizontalOverflow(page);
  await shot(page, 'question', info.project.name);
  const sent = page.waitForRequest((r) => r.method() === 'POST' && r.url().includes(`/runtime-sessions/${id}/requests/`));
  await send.click();
  await sent;
  await expect(dock).toHaveCount(0, { timeout: 20_000 });
});
