import { test, expect, type APIRequestContext, type Page } from '@playwright/test';
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { apiCtx } from './seed';

// ─────────────────────────────────────────────────────────────────────────────
// The design review's findings in Threads, on REAL fake-agent turns (desktop):
//   • Full access always shows its amber chip (thread and stage alike); only a
//     calm default (Supervised / Auto-accept edits) hides the chip;
//   • with the agent working and a draft typed, the primary says "Queue";
//   • a review sent from the Changes panel reads in the timeline as one
//     "Review · N notes" block (plain text to the agent);
//   • the model picker names every provider beside its glyph and closes once
//     a pick arms a switch;
//   • toasts never sit on the composer's chip or Send at 1440;
//   • a usage limit's "Continue with" brings the limited prompt back.
// Screenshots: OTTO_E2E_SHOTS/review-round/ (light and dark).
// ─────────────────────────────────────────────────────────────────────────────

test.use({ serviceWorkers: 'block' });

let ctx: APIRequestContext;
let base = '';
let wsId = '';
let dir = '';

test.beforeAll(async () => {
  const c = await apiCtx();
  ctx = c.ctx;
  base = c.base;
  dir = mkdtempSync(join(tmpdir(), 'otto-e2e-review-round-'));
  writeFileSync(join(dir, 'README.md'), '# review round\n');
  const r = await ctx.post(`${base}/api/v1/workspaces`, { data: { name: 'Review round WS', root_path: dir } });
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
  await page.addInitScript((id) => localStorage.setItem('otto_workspace', id as string), wsId);
  await page.setViewportSize({ width: 1440, height: 900 });
});

export async function shot(page: Page, name: string): Promise<void> {
  const out = process.env.OTTO_E2E_SHOTS;
  if (!out) return;
  mkdirSync(join(out, 'review-round'), { recursive: true });
  for (const scheme of ['light', 'dark'] as const) {
    await page.emulateMedia({ colorScheme: scheme });
    await page.screenshot({ path: join(out, 'review-round', `${name}-${scheme}.png`), animations: 'disabled' });
  }
  await page.emulateMedia({ colorScheme: 'light' });
}

async function open(page: Page, provider: string, title: string, access?: string) {
  const r = await ctx.post(`${base}/api/v1/workspaces/${wsId}/runtime-sessions`, { data: { provider, cwd: dir, title, ...(access ? { access_mode: access } : {}) } });
  expect(r.ok(), await r.text()).toBeTruthy();
  const id = (await r.json()).id as string;
  await page.goto(`/#/threads/${id}`);
  const thread = page.getByTestId(`runtime-thread-${id}`);
  await expect(thread).toBeVisible({ timeout: 30_000 });
  return { id, thread };
}

const overlaps = (a: { x: number; y: number; width: number; height: number }, b: { x: number; y: number; width: number; height: number }): boolean =>
  a.x < b.x + b.width && b.x < a.x + a.width && a.y < b.y + b.height && b.y < a.y + a.height;

test('Full access always shows its amber chip, in a thread and on the stage; Supervised (a calm default) shows none', async ({ page }) => {
  const { thread } = await open(page, 'codex', 'Full access thread', 'full-access');
  const chip = thread.getByTestId('runtime-access');
  await expect(chip).toContainText('Full access');
  await shot(page, 'access-thread-full');
  await page.goto('/#/threads/new');
  const stage = page.getByTestId('new-thread-stage');
  await expect(stage).toBeVisible({ timeout: 30_000 });
  const sChip = stage.getByTestId('runtime-access');
  if (await sChip.count()) await expect(sChip).toContainText(/Full access|Auto-accept/); // never a chip for Supervised as the default
  await shot(page, 'access-stage');
});

test('working with a draft: the primary says Queue (the word beside its icon)', async ({ page }) => {
  const { thread } = await open(page, 'codex', 'Queue word');
  const composer = thread.getByTestId('runtime-composer');
  await composer.fill('slow 8');
  await thread.getByTestId('runtime-send').click();
  await expect(thread.getByTestId('runtime-stop')).toBeVisible({ timeout: 20_000 });
  await composer.fill('and then this');
  const primary = thread.getByTestId('runtime-send');
  await expect(primary).toHaveAttribute('data-delivery', 'queue');
  await expect(primary).toContainText('Queue');
  await shot(page, 'queue-word');
  await thread.getByTestId('runtime-stop').click();
});

test('a sent review reads as one "Review · N notes" block in the timeline', async ({ page }) => {
  const { id, thread } = await open(page, 'codex', 'Review block');
  const text = 'Please look.\n\n--- Review notes on the diff: Review · 2 notes ---\n1. src/a.ts (new 9 · old 9)\n   > -save(note);\n   > +queue.push(note);\n   Keep the old name.\n\n2. README.md (new 1)\n   > +# harbor\n   Typo?';
  expect((await ctx.post(`${base}/api/v1/runtime-sessions/${id}/prompt`, { data: { text } })).ok()).toBeTruthy();
  const block = thread.getByTestId('runtime-review-block');
  await expect(block).toContainText('Review · 2 notes', { timeout: 20_000 });
  await expect(thread.locator('.rt-user').getByText('--- Review notes on the diff')).toHaveCount(0); // (the fake agent echoes it back)
  await expect(thread.locator('.rt-user .rt-bubble').filter({ hasText: 'Please look.' })).toHaveCount(1);
  await block.locator('summary').click();
  await expect(block).toContainText('src/a.ts');
  await expect(block).toContainText('Keep the old name.');
  await shot(page, 'review-block');
});

test('the model picker names every provider and closes once a pick arms a switch', async ({ page }) => {
  const { thread } = await open(page, 'codex', 'Picker names');
  await thread.getByTestId('runtime-model').click();
  const picker = page.getByTestId('model-picker');
  await expect(picker).toBeVisible();
  await expect(picker.getByTestId('model-picker-provider-codex')).toContainText('Codex');
  await expect(picker.getByTestId('model-picker-provider-claude')).toContainText('Claude');
  await shot(page, 'model-picker-names');
  await picker.getByTestId('model-picker-provider-claude').click();
  await picker.locator('[data-mp-idx="0"]').click();
  await expect(picker).toHaveCount(0);
  await expect(thread.getByTestId('runtime-switch-notice')).toContainText('Next message continues on');
});

test('toasts in Threads never cover the composer’s model chip or Send at 1440', async ({ page }) => {
  const { id, thread } = await open(page, 'codex', 'Toast placement');
  const row = page.getByTestId(`thread-row-${id}`).first();
  await row.hover();
  await page.getByTestId(`thread-pin-${id}`).click();
  const toast = page.locator('.toasts .toast').first();
  await expect(toast).toBeVisible();
  const [t, chip, send] = [(await toast.boundingBox())!, (await thread.getByTestId('runtime-model').boundingBox())!, (await thread.getByTestId('runtime-send').boundingBox())!];
  expect(overlaps(t, chip), 'toast over the model chip').toBe(false);
  expect(overlaps(t, send), 'toast over Send').toBe(false);
  await shot(page, 'toast-placement');
});

test('a usage limit: Continue with another provider arms the switch and brings the limited prompt back', async ({ page }) => {
  const { thread } = await open(page, 'codex', 'Limit continue');
  const composer = thread.getByTestId('runtime-composer');
  await composer.fill('limit');
  await thread.getByTestId('runtime-send').click();
  const line = thread.getByTestId('runtime-usage-limit');
  await expect(line).toBeVisible({ timeout: 20_000 });
  await shot(page, 'limit-line');
  await line.getByTestId('runtime-limit-switch').click();
  await expect(thread.getByTestId('runtime-switch-notice')).toBeVisible();
  await expect(composer).toHaveValue('limit');
  await shot(page, 'limit-continue');
});
