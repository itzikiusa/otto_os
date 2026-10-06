import { test, expect, type Page } from '@playwright/test';
import { apiCtx, seedShellSession, seedWorkspace } from './seed';
import { expectFullyInViewport, expectNoHorizontalOverflow } from './helpers';

// Home → Classrooms widget: workspaces as classrooms, sessions as students,
// the user as the headmaster. The 3D canvas needs WebGL (headless Chromium may
// or may not have it), so every assertion goes through the accessible
// companion list, which exists in both modes: a visible list in List view /
// without WebGL, a keyboard-reachable one in 3D view. Kick-out = delete via
// the app's delete path, behind a danger confirm.

test.describe.configure({ mode: 'serial' });

let wsId = '';
// Every spec's default workspace is "E2E WS" on the shared daemon, and the box
// shows one classroom per workspace — a unique name keeps ours addressable.
const WS_NAME = `Classrooms E2E ${Date.now().toString(36)}`;
let openId = '';
let kickId = '';
let base = '';
let ctx: Awaited<ReturnType<typeof apiCtx>>['ctx'];

async function boot(page: Page): Promise<void> {
  await page.addInitScript((id) => {
    localStorage.setItem('otto_workspace', id as string);
    if (sessionStorage.getItem('otto_e2e_classrooms_reset')) return;
    sessionStorage.setItem('otto_e2e_classrooms_reset', '1');
    localStorage.removeItem('otto_home_active');
    localStorage.removeItem('otto_bar_spaces');
    localStorage.removeItem('otto_home_rotate');
    // One space holding just the Classrooms widget.
    localStorage.setItem(
      'otto_home_views',
      JSON.stringify([{ id: 'v1', name: 'Campus', boxes: [{ id: 'cr1', kind: 'classrooms', w: 12, h: 6, config: {} }] }]),
    );
  }, wsId);
  await page.goto('/#/home');
  await expect(page.locator('.shell')).toBeVisible({ timeout: 15_000 });
}

const box = (page: Page) => page.locator('section.hbox[data-kind="classrooms"]');
const list = (page: Page) => box(page).getByRole('region', { name: 'Classrooms list' });
/** OUR classroom: other specs on the shared daemon seed students with the
 *  same titles ("E2E Shell") in their own workspaces. */
const ours = (page: Page) => list(page).getByRole('region', { name: `Classroom ${WS_NAME}`, exact: true });

/** Show the visible list (List view) — a no-op without WebGL (already a list). */
async function listView(page: Page): Promise<void> {
  const toggle = box(page).getByRole('button', { name: 'List', exact: true });
  if (await toggle.count()) {
    // Wait for the switch to TAKE: the 3D view keeps the same list as an
    // sr-only companion (still "visible" to Playwright) under the box's bar,
    // so a click lost to the first paint would leave every row unhoverable.
    await expect(async () => {
      if ((await toggle.getAttribute('aria-pressed')) !== 'true') await toggle.click();
      await expect(toggle).toHaveAttribute('aria-pressed', 'true', { timeout: 3_000 });
      await expect(list(page)).not.toHaveClass(/sr-only/, { timeout: 1_000 });
    }).toPass({ timeout: 30_000 }); // a busy shared daemon lists many classrooms
  }
  await expect(list(page)).toBeVisible();
}

test.beforeAll(async () => {
  const c = await apiCtx();
  ctx = c.ctx;
  base = c.base;
  wsId = await seedWorkspace(c.ctx, c.base, WS_NAME);
  openId = await seedShellSession(c.ctx, c.base, wsId);
  const r = await c.ctx.post(`${c.base}/api/v1/workspaces/${wsId}/sessions`, {
    data: { kind: 'agent', provider: 'shell', title: 'E2E Kick', cwd: '/tmp', meta: { origin: 'manual' } },
  });
  kickId = ((await r.json()) as { id: string }).id;
});

test.beforeEach(async ({}, info) => {
  test.skip(info.project.name !== 'desktop-browser', 'desktop-browser project only');
});

test('the Add widget picker offers Classrooms', async ({ page }) => {
  await boot(page);
  await page.getByRole('button', { name: 'Add widget' }).click();
  await expect(page.getByRole('dialog').getByRole('button', { name: /Classrooms/ })).toBeVisible();
  await page.keyboard.press('Escape');
});

test('the box renders classrooms with the seeded students', async ({ page }) => {
  await boot(page);
  await expect(box(page)).toBeVisible();
  // Settles: either the 3D stage (with the headmaster label once drawn) or the list.
  await expect(box(page).locator('.stage, .list:not(.sr-only)').first()).toBeVisible();
  await expect(box(page).locator('.sum')).toContainText(/classroom.*student/);
  await listView(page);
  const room = ours(page);
  await expect(room).toBeVisible();
  await expect(room.getByRole('button', { name: /^Open E2E Shell/ })).toBeVisible();
  await expect(room.getByRole('button', { name: /^Open E2E Kick/ })).toBeVisible();
  await expectNoHorizontalOverflow(page);
});

test('focus / hover shows a tooltip fully inside the viewport', async ({ page }) => {
  await boot(page);
  await listView(page);
  const row = ours(page).getByRole('button', { name: /^Open E2E Shell/ });
  await row.hover();
  const tip = page.getByRole('tooltip');
  await expect(tip).toContainText('E2E Shell');
  await expect(tip).toContainText('Shell');
  await expect(tip).toContainText(WS_NAME);
  await expectFullyInViewport(page, tip, 'classrooms tooltip');
  // A short window: the tooltip still clamps inside.
  await page.setViewportSize({ width: 820, height: 420 });
  await row.scrollIntoViewIfNeeded();
  await row.hover();
  await expectFullyInViewport(page, page.getByRole('tooltip'), 'classrooms tooltip (short window)');
  // Keyboard path in 3D view: focusing a student shows its tooltip too.
  const threeD = box(page).getByRole('button', { name: '3D', exact: true });
  if (await threeD.count()) {
    await threeD.click();
    await ours(page).getByRole('button', { name: /^Open E2E Shell/ }).focus();
    await expect(page.getByRole('tooltip')).toContainText('E2E Shell');
    await expectFullyInViewport(page, page.getByRole('tooltip'), 'classrooms tooltip (focus)');
  }
});

test('clicking a student opens that session', async ({ page }) => {
  await boot(page);
  await listView(page);
  await ours(page).getByRole('button', { name: /^Open E2E Shell/ }).click();
  await expect(page).toHaveURL(new RegExp(`#/agents/${openId}$`));
});

test('the headmaster kicks a student out: danger confirm, then the session is deleted', async ({ page }) => {
  await boot(page);
  await listView(page);
  // Cancel first: nothing is deleted.
  await ours(page).getByRole('button', { name: 'More actions for E2E Kick' }).click();
  await page.getByRole('menuitem', { name: /Kick out/ }).click();
  const dlg = page.getByRole('dialog');
  await expect(dlg).toContainText('E2E Kick');
  await expect(dlg).toContainText(WS_NAME);
  await expect(dlg).toContainText('entire history');
  await dlg.getByRole('button', { name: 'Cancel' }).click();
  expect((await ctx.get(`${base}/api/v1/sessions?ids=${kickId}`)).ok()).toBe(true);
  await expect(ours(page).getByRole('button', { name: /^Open E2E Kick/ })).toBeVisible();

  // Confirm: the student leaves, the toast says so, the row is gone server-side.
  await ours(page).getByRole('button', { name: 'More actions for E2E Kick' }).click();
  await page.getByRole('menuitem', { name: /Kick out/ }).click();
  await page.getByRole('dialog').getByRole('button', { name: 'Kick out', exact: true }).click();
  await expect(page.locator('.toast').filter({ hasText: 'Kicked out E2E Kick' })).toBeVisible();
  await expect(ours(page).getByRole('button', { name: /^Open E2E Kick/ })).toHaveCount(0);
  await expect
    .poll(async () => {
      const r = await ctx.get(`${base}/api/v1/sessions?ids=${kickId}`);
      return r.ok() ? ((await r.json()) as unknown[]).length : -1;
    })
    .toBe(0);
});

test('detention archives a student (resumable)', async ({ page }) => {
  await boot(page);
  await listView(page);
  await ours(page).getByRole('button', { name: 'More actions for E2E Shell' }).click();
  await page.getByRole('menuitem', { name: /detention/ }).click();
  await expect(page.locator('.toast').filter({ hasText: 'Session archived' })).toBeVisible();
  await expect(ours(page).getByRole('button', { name: /^Open E2E Shell/ })).toHaveCount(0);
});
