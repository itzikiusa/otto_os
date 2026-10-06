import { test, expect, type Page } from '@playwright/test';
import { apiCtx, seedWorkspace } from './seed';
import { expectFullyInViewport } from './helpers';

// Home → Classrooms, the 3D path (S14-12). desktop-home-classrooms.spec.ts
// kicks out in List view, where no scene exists and the "animation" resolves
// at once — which is how S14-01 shipped: in 3D the session_removed refetch
// drops the student mid-walk and the kick never resolved (no toast). Here
// Chromium gets a software WebGL (SwiftShader) and motion stays ON, so the
// walk-out really runs while the refetch lands.

test.use({
  launchOptions: { args: ['--use-angle=swiftshader', '--enable-unsafe-swiftshader', '--ignore-gpu-blocklist'] },
  reducedMotion: 'no-preference',
});
test.describe.configure({ mode: 'serial' });

let wsId = '';
let kickId = '';
let base = '';
let ctx: Awaited<ReturnType<typeof apiCtx>>['ctx'];

async function boot(page: Page): Promise<void> {
  await page.addInitScript((id) => {
    localStorage.setItem('otto_workspace', id as string);
    if (sessionStorage.getItem('otto_e2e_classrooms3d_reset')) return;
    sessionStorage.setItem('otto_e2e_classrooms3d_reset', '1');
    localStorage.removeItem('otto_home_active');
    localStorage.removeItem('otto_bar_spaces');
    localStorage.removeItem('otto_home_rotate');
    localStorage.setItem(
      'otto_home_views',
      JSON.stringify([{ id: 'v1', name: 'Campus', boxes: [{ id: 'cr1', kind: 'classrooms', w: 12, h: 6, config: { view: '3d' } }] }]),
    );
  }, wsId);
  await page.goto('/#/home');
  await expect(page.locator('.shell')).toBeVisible({ timeout: 15_000 });
}

const box = (page: Page) => page.locator('section.hbox[data-kind="classrooms"]');
const list = (page: Page) => box(page).getByRole('region', { name: 'Classrooms list' });
const card = (page: Page) => page.getByRole('group', { name: /— details$/ });

test.beforeAll(async () => {
  const c = await apiCtx();
  ctx = c.ctx;
  base = c.base;
  wsId = await seedWorkspace(c.ctx, c.base);
  const r = await c.ctx.post(`${c.base}/api/v1/workspaces/${wsId}/sessions`, {
    data: { kind: 'agent', provider: 'shell', title: 'E2E Kick 3D', cwd: '/tmp', meta: { origin: 'manual' } },
  });
  kickId = ((await r.json()) as { id: string }).id;
});

test.beforeEach(async ({}, info) => {
  test.skip(info.project.name !== 'desktop-browser', 'desktop-browser project only');
});

test('3D: the scene mounts on software WebGL', async ({ page }) => {
  await boot(page);
  await expect(box(page).locator('.stage canvas')).toBeVisible({ timeout: 20_000 });
  await expect(box(page).locator('.overlay .you')).toBeAttached();
  await expect(box(page).getByText('3D isn’t available here')).toHaveCount(0);
});

test('3D: every keyboard stop in a row shows the card (visible focus)', async ({ page }) => {
  await boot(page);
  await expect(box(page).locator('.stage canvas')).toBeVisible({ timeout: 20_000 });
  await list(page).getByRole('button', { name: /^Open E2E Kick 3D/ }).focus();
  await expect(card(page)).toContainText('E2E Kick 3D');
  await page.keyboard.press('Tab'); // → the row's "More actions"
  await expect(list(page).getByRole('button', { name: 'More actions for E2E Kick 3D' })).toBeFocused();
  await expect(card(page)).toContainText('E2E Kick 3D');
  await expectFullyInViewport(page, card(page), 'classrooms card (⋯ focus)');
});

test('3D: kick-out from the card walks out, then toasts — even when the refetch lands mid-walk', async ({ page }) => {
  await boot(page);
  await expect(box(page).locator('.stage canvas')).toBeVisible({ timeout: 20_000 });
  await list(page).getByRole('button', { name: /^Open E2E Kick 3D/ }).focus();
  await card(page).getByRole('button', { name: 'Kick out…' }).click();
  await page.getByRole('dialog').getByRole('button', { name: 'Kick out', exact: true }).click();
  // The 1.4 s walk outlasts the DELETE + session_removed refetch (~350 ms):
  // before the fix the kick never resolved and this toast never came.
  await expect(page.locator('.toast').filter({ hasText: 'Kicked out E2E Kick 3D' })).toBeVisible({ timeout: 10_000 });
  await expect(list(page).getByRole('button', { name: /^Open E2E Kick 3D/ })).toHaveCount(0);
  await expect
    .poll(async () => {
      const r = await ctx.get(`${base}/api/v1/sessions?ids=${kickId}`);
      return r.ok() ? ((await r.json()) as unknown[]).length : -1;
    })
    .toBe(0);
});
