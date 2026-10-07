import { test, expect, type Page } from '@playwright/test';
import { apiCtx, seedWorkspace } from './seed';
import { randomUUID } from 'node:crypto';
import { expectFullyInViewport, expectNoHorizontalOverflow } from './helpers';

test.use({ launchOptions: { args: ['--disable-webgl'] }, serviceWorkers: 'block' });
let wsId = '';
const wsName = `School fallback ${randomUUID()}`;
const box = (page: Page) => page.locator('section.hbox[data-kind="classrooms"]');

test.beforeAll(async () => {
  const { ctx, base } = await apiCtx();
  wsId = await seedWorkspace(ctx, base, wsName);
  await ctx.dispose();
});

async function boot(page: Page) {
  await page.addInitScript((id) => {
    localStorage.setItem('otto_workspace', id);
    localStorage.removeItem('otto_home_active');
    localStorage.removeItem('otto_bar_spaces');
    localStorage.removeItem('otto_home_rotate');
    localStorage.setItem('otto_home_views', JSON.stringify([{ id: 'v1', name: 'School', boxes: [{ id: 'cr1', kind: 'classrooms', w: 12, h: 8, config: {} }] }]));
  }, wsId);
  await page.goto('/#/home');
  await expect(box(page)).toBeVisible();
}

test('initial School loading and failed data use truthful load states and Retry', async ({ page }) => {
  // This fixture tests manual recovery. Other workers' session events trigger
  // School's live refresh and can recover the API before Retry is clicked.
  // Keep the event connection open but quiet; HTTP loading and Retry still use
  // the production component and request path against the fixture below.
  await page.routeWebSocket('**/ws/events*', () => {});
  let release!: () => void;
  const gate = new Promise<void>((resolve) => { release = resolve; });
  let failed = true;
  let requests = 0;
  await page.route('**/api/v1/sessions?archived=false&limit=1000', async (route) => {
    requests++;
    await gate;
    await route.fulfill({ status: failed ? 503 : 200, contentType: 'application/json', body: failed ? JSON.stringify({ error: 'School fixture unavailable' }) : '[]' });
  });
  await boot(page);
  try { await expect(box(page).getByRole('status', { name: 'Loading the school' })).toBeVisible(); } finally { release(); }
  await expect(box(page).getByRole('alert')).toContainText('Couldn’t load the school');
  await expect(box(page)).not.toContainText('last good load');
  const beforeRetry = requests;
  failed = false;
  await box(page).getByRole('button', { name: 'Retry', exact: true }).click();
  await expect.poll(() => requests).toBeGreaterThan(beforeRetry);
  await expect(box(page).getByRole('alert')).toHaveCount(0);
  await expect(box(page).getByRole('region', { name: 'School list', exact: true })).toBeVisible();
});

for (const viewport of [{ width: 1440, height: 900 }, { width: 390, height: 844 }]) {
  test(`no-WebGL list retains overflow sessions and menus at ${viewport.width}px`, async ({ page }) => {
    await page.setViewportSize(viewport);
    await page.addInitScript((phone) => {
      localStorage.setItem('otto_scheme', phone ? 'dark' : 'light');
      localStorage.setItem('otto_direction', phone ? 'rtl' : 'ltr');
    }, viewport.width < 640);
    const sessions = Array.from({ length: 46 }, (_, i) => ({
      id: `school-overflow-${i}`, workspace_id: wsId, kind: 'agent', provider: 'shell',
      title: `Overflow student ${i}`, status: 'running', cwd: '/tmp', archived: false,
      created_at: new Date(Date.UTC(2026, 0, 1, 0, i)).toISOString(), last_active_at: '2026-01-01T01:00:00Z',
      meta: i >= 38 ? { source: 'workflow' } : {},
    }));
    await page.route('**/api/v1/sessions?archived=false&limit=1000', (route) => route.fulfill({ json: sessions }));
    await boot(page);
    await expect(box(page)).toContainText('WebGL is off');
    const school = box(page).getByRole('region', { name: 'School list', exact: true });
    await expect(school).toBeVisible();
    // School also merges live sessions from other workers' workspaces. Count
    // every fixture member in our classroom, including the eight back-row kids.
    const list = school.getByRole('region', { name: `Classroom ${wsName}`, exact: true });
    await expect(list.locator('.lrow')).toHaveCount(46);
    const last = list.getByRole('button', { name: /^Open Overflow student 45,/ });
    await last.focus();
    await expectFullyInViewport(page, last, 'overflow student');
    await page.keyboard.press('Tab');
    const more = list.getByRole('button', { name: 'More actions for Overflow student 45' });
    await expect(more).toBeFocused();
    await more.press('Enter');
    await expect(page.getByRole('menuitem', { name: 'Open session', exact: true })).toBeVisible();
    await page.keyboard.press('Escape');
    await expectNoHorizontalOverflow(page);
    await box(page).screenshot({ path: test.info().outputPath(`school-list-${viewport.width}.png`) });
    await last.press('Enter');
    await expect(page).toHaveURL(/#\/agents\/school-overflow-45$/);
  });
}

test('complete classroom list pages over one hundred rows and clamps after refresh', async ({ page }) => {
  let sessions = Array.from({ length: 106 }, (_, i) => ({
    id: `school-page-${i}`, workspace_id: wsId, kind: 'agent', provider: 'shell',
    title: `Paged student ${i}`, status: 'running', cwd: '/tmp', archived: false,
    created_at: new Date(Date.UTC(2026, 0, 1, 0, i)).toISOString(), last_active_at: '2026-01-01T02:00:00Z', meta: {},
  }));
  await page.route('**/api/v1/sessions?archived=false&limit=1000', (route) => route.fulfill({ json: sessions }));
  await boot(page);
  const room = box(page).getByRole('region', { name: `Classroom ${wsName}`, exact: true });
  await expect(room.locator('.lrow')).toHaveCount(100);
  await expect(room.getByRole('button', { name: `Previous sessions in ${wsName}`, exact: true })).toBeDisabled();
  await room.getByRole('button', { name: `Next sessions in ${wsName}`, exact: true }).click();
  await expect(room.locator('.lrow')).toHaveCount(6);
  await expect(room.getByRole('button', { name: `Next sessions in ${wsName}`, exact: true })).toBeDisabled();
  await room.getByRole('button', { name: 'More actions for Paged student 105' }).click();
  await expect(page.getByRole('menuitem', { name: 'Open session', exact: true })).toBeVisible();
  await page.keyboard.press('Escape');
  await room.getByRole('button', { name: `Previous sessions in ${wsName}`, exact: true }).click();
  await expect(room.locator('.lrow')).toHaveCount(100);
  await room.getByRole('button', { name: `Next sessions in ${wsName}`, exact: true }).click();
  sessions = sessions.slice(0, 2);
  await box(page).getByRole('button', { name: 'Refresh Classrooms', exact: true }).click();
  await expect(room.locator('.lrow')).toHaveCount(2);
  await expect(room.getByRole('button', { name: /^Open Paged student 0,/ })).toBeVisible();
  await expect(room.getByRole('navigation', { name: `Sessions in classroom ${wsName}`, exact: true })).toHaveCount(0);
});

test('failed School refresh retains the loaded list and exposes Retry until recovery', async ({ page }) => {
  // As above, recover through Retry rather than another worker's live event.
  await page.routeWebSocket('**/ws/events*', () => {});
  let failed = false;
  let title = 'Retained student';
  let requests = 0;
  await page.route('**/api/v1/sessions?archived=false&limit=1000', (route) => {
    requests++;
    return route.fulfill(failed ? {
      status: 503, contentType: 'application/json', body: JSON.stringify({ error: 'School refresh unavailable' }),
    } : {
      json: [{ id: 'school-refresh-kid', workspace_id: wsId, kind: 'agent', provider: 'shell', title,
        status: 'running', cwd: '/tmp', archived: false, meta: {},
        created_at: '2026-01-01T00:00:00Z', last_active_at: '2026-01-01T00:00:00Z' }],
    });
  });
  await boot(page);
  const school = box(page).getByRole('region', { name: 'School list', exact: true });
  await expect(school).toBeVisible();
  const list = school.getByRole('region', { name: `Classroom ${wsName}`, exact: true });
  const retained = list.getByRole('button', { name: /^Open Retained student,/ });
  await expect(retained).toBeVisible();
  failed = true;
  await box(page).getByRole('button', { name: 'Refresh Classrooms', exact: true }).click();
  const stale = box(page).getByTestId('load-stale');
  await expect(stale).toContainText('Couldn’t refresh the school');
  await expect(retained).toBeVisible();
  await expect(list.locator('.lrow')).toHaveCount(1);
  await expect(box(page).getByTestId('load-error')).toHaveCount(0);
  const beforeRetry = requests;
  failed = false;
  title = 'Recovered student';
  await stale.getByRole('button', { name: 'Retry', exact: true }).click();
  await expect.poll(() => requests).toBeGreaterThan(beforeRetry);
  await expect(stale).toHaveCount(0);
  await expect(list.getByRole('button', { name: /^Open Recovered student,/ })).toBeVisible();
  await expect(retained).toHaveCount(0);
});
