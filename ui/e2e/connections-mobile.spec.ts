import { test, expect, type Page, type APIRequestContext } from '@playwright/test';
import { apiCtx, seedWorkspace } from './seed';
import { mockDbRoutes, seedMockDbConnection } from './db-mock';
import { expectFullyInViewport } from './helpers';

// Connections list + DB workbench — PHONE/responsive usability guard. Runs on
// every device project (phones + tablets). The list is the page (one inbox
// row per profile: name … state word, kind · host · section); one filter
// menu and a search narrow it; touch targets are finger-sized; the workbench
// puts the rows on screen with Tables as a sheet that Back closes.

// The PWA service worker would take the engine calls past page.route.
test.use({ serviceWorkers: 'block' });

let workspaceId = '';
let prodSectionId = '';
let globalDbId = '';
const globalDbName = `mobile-global-db-${Math.random().toString(36).slice(2, 8)}`;

async function seedSection(ctx: APIRequestContext, base: string, wsId: string, name: string): Promise<string> {
  const r = await ctx.post(`${base}/api/v1/workspaces/${wsId}/connection-sections`, { data: { name } });
  if (!r.ok()) throw new Error(`seed section → ${r.status()} ${await r.text()}`);
  return (await r.json()).id as string;
}

async function seedSsh(
  ctx: APIRequestContext,
  base: string,
  wsId: string,
  name: string,
  sectionId: string | null,
  environment = 'prod',
): Promise<void> {
  const r = await ctx.post(`${base}/api/v1/workspaces/${wsId}/connections`, {
    data: {
      name,
      kind: 'ssh',
      params: { host: 'server.really-long-hostname.example.com', port: 22, user: 'deploy-user' },
      secret: null,
      environment,
      read_only: false,
      section_id: sectionId,
    },
  });
  if (!r.ok()) throw new Error(`seed ssh → ${r.status()} ${await r.text()}`);
}

async function seedCustom(ctx: APIRequestContext, base: string, wsId: string, sectionId: string | null): Promise<void> {
  const r = await ctx.post(`${base}/api/v1/workspaces/${wsId}/connections`, {
    data: {
      name: 'my-custom-cli',
      kind: 'custom',
      params: { command_template: 'psql -h {host} -U {user} {db}' },
      secret: null,
      environment: 'dev',
      read_only: true,
      section_id: sectionId,
    },
  });
  if (!r.ok()) throw new Error(`seed custom → ${r.status()} ${await r.text()}`);
}

test.beforeAll(async () => {
  const { ctx, base } = await apiCtx();
  workspaceId = await seedWorkspace(ctx, base);
  prodSectionId = await seedSection(ctx, base, workspaceId, 'Production servers');
  await seedSection(ctx, base, workspaceId, 'Empty staging folder');
  await seedSsh(ctx, base, workspaceId, 'prod-web-bastion', prodSectionId, 'prod');
  await seedSsh(ctx, base, workspaceId, 'ungrouped-host', null, 'staging');
  await seedCustom(ctx, base, workspaceId, prodSectionId);
  // DB profiles are global: a connection created in another workspace must
  // still contribute a type chip and participate in this workspace's filters.
  const otherWorkspace = await seedWorkspace(ctx, base);
  globalDbId = await seedMockDbConnection(ctx, base, otherWorkspace, globalDbName, 'mysql');
  await ctx.dispose();
});

// Activate the seeded workspace (so connections load), close the nav drawer
// (defaults open on a fresh phone profile), and pin the type filter to All so a
// previous test's chip choice can't hide seeded rows.
test.beforeEach(async ({ page }) => {
  await page.addInitScript((wsId) => {
    localStorage.setItem('otto_workspace', wsId as string);
    localStorage.setItem('otto_rail_expanded', '0');
    localStorage.setItem('otto_connhub_filter', 'all');
  }, workspaceId);
});

async function gotoPage(page: Page): Promise<void> {
  await page.goto('/#/connections');
  await expect(page.locator('.conn-row').first()).toBeVisible({ timeout: 30_000 });
}

/** Neither the document nor the page body scrolls sideways (wide grids
 *  scroll inside their own block). */
async function assertFitsWidth(page: Page): Promise<void> {
  const r = await page.evaluate(() => {
    const de = document.documentElement;
    const body = document.querySelector<HTMLElement>('.page-body');
    return { docScrollW: de.scrollWidth, docClientW: de.clientWidth, bodyOver: body ? body.scrollWidth - body.clientWidth : 0 };
  });
  expect(r.docScrollW).toBeLessThanOrEqual(r.docClientW + 1);
  expect(r.bodyOver).toBeLessThanOrEqual(1);
}

const row = (page: Page, name: string) => page.locator('.conn-row', { hasText: name }).first();

test.describe('connections list — responsive', () => {
  test('every seeded profile renders as a row in words and fits the viewport', async ({ page }) => {
    await gotoPage(page);
    await expect(row(page, 'prod-web-bastion')).toContainText('SSH');
    await expect(row(page, 'prod-web-bastion')).toContainText('Production');
    await expect(row(page, 'prod-web-bastion')).toContainText('Not checked');
    await expect(row(page, 'ungrouped-host')).toBeVisible();
    await expect(row(page, 'my-custom-cli')).toContainText('read-only');
    // Under its section's heading a row does not repeat the section's name.
    await expect(row(page, 'my-custom-cli')).not.toContainText('Production servers');
    await expect(page.getByRole('region', { name: 'Production servers' }).first()).toBeVisible();
    await expect(page.getByRole('region', { name: 'Empty staging folder' }).first()).toBeVisible();
    await expect(page.getByRole('region', { name: 'Ungrouped' })).toBeVisible();
    await assertFitsWidth(page);
  });

  test('the filter menu and the search narrow the list', async ({ page }) => {
    await gotoPage(page);
    await page.getByTestId('conn-filter').click();
    await page.getByRole('menuitemcheckbox', { name: 'MySQL', exact: true }).click();
    await expect(row(page, globalDbName)).toBeVisible();
    await expect(page.locator('.conn-row', { hasText: 'prod-web-bastion' })).toHaveCount(0);
    await page.getByRole('button', { name: 'Clear the MySQL filter' }).click();
    await page.getByRole('searchbox', { name: 'Search connections' }).fill('bastion');
    await expect(row(page, 'prod-web-bastion')).toBeVisible();
    await expect(page.locator('.conn-row', { hasText: 'my-custom-cli' })).toHaveCount(0);
    await assertFitsWidth(page);
  });

  test('touch targets on the list are at least 44 px', async ({ page }, info) => {
    test.skip(!/iphone|android/.test(info.project.name), 'phone projects');
    await gotoPage(page);
    const small = await page.evaluate(() =>
      [...document.querySelectorAll<HTMLElement>('.page-body button, .page-body input, .page-header-bar button')]
        .filter((el) => { const r = el.getBoundingClientRect(); return r.width > 0 && r.height > 0 && r.height < 43.5; })
        .map((el) => (el.getAttribute('aria-label') ?? el.textContent ?? el.tagName).trim().slice(0, 30)),
    );
    expect(small).toEqual([]);
  });

  test('phone flow: a row opens the workbench, a run puts the rows on screen, Tables is a sheet Back closes', async ({ page }, info) => {
    test.skip(!/iphone|android/.test(info.project.name), 'phone projects');
    await mockDbRoutes(page, globalDbId);
    await gotoPage(page);
    await row(page, globalDbName).click();
    await expect(page.getByTestId('db-conn-state')).toBeVisible({ timeout: 20_000 });
    const editor = page.locator('.qe-edit .cm-content');
    await editor.click();
    await page.keyboard.press('ControlOrMeta+A');
    await page.keyboard.insertText('SELECT * FROM shop.orders LIMIT 12');
    await page.locator('.btn.small.primary', { hasText: 'Run' }).first().click();
    await expect(page.locator('.grid tbody tr').first()).toBeVisible();
    // The query tabs' + takes a finger (design review 2, H8).
    const plus = (await page.getByRole('button', { name: 'New query tab' }).boundingBox())!;
    expect(Math.min(plus.width, plus.height), 'New query tab').toBeGreaterThanOrEqual(44);
    // Row numbers from 10 up stay whole beside the touch "+" (design review 2, N8: row 10 read "1").
    const tenth = page.locator('.grid tbody tr:not(.spacer)').nth(9);
    await expect(tenth.locator('.rownum-n')).toHaveText('10');
    const [num, dup] = await Promise.all([tenth.locator('.rownum-n').boundingBox(), tenth.locator('.row-dup').boundingBox()]);
    expect(num!.x + num!.width, 'the number ends before the +').toBeLessThanOrEqual(dup!.x + 0.5);
    // The editor folds to its one line; the rows are what is on screen.
    await expect(page.locator('.qe-edit')).toBeHidden();
    await expect(page.getByRole('button', { name: 'Edit the query' })).toBeVisible();
    // …down to the bottom bar (only the row count line under them), not a
    // block that stops halfway with the rest of the screen empty.
    const [res, body] = await Promise.all([page.locator('.qe-results').boundingBox(), page.locator('.page-body').boundingBox()]);
    expect(body!.y + body!.height - (res!.y + res!.height)).toBeLessThan(40);
    // No keyboard hints on touch: not on Run, not in the menus.
    await expect(page.getByTestId('page-header')).not.toContainText('⌘', { useInnerText: true });
    // The results toolbar keeps the view switch and the filter; the rest is in
    // its one ⋯ — every control finger-sized.
    const bar = page.locator('.grid-toolbar');
    await expect(bar.getByRole('button', { name: /Export/ })).toHaveCount(0);
    const small = await bar.evaluate((el) =>
      [...el.querySelectorAll<HTMLElement>('button, input')].filter((b) => b.getBoundingClientRect().height < 40).map((b) => b.getAttribute('aria-label') ?? b.textContent),
    );
    expect(small).toEqual([]);
    await bar.getByRole('button', { name: 'More result actions' }).click();
    const menu = page.getByRole('menu');
    await expect(menu.getByRole('menuitem', { name: /^Download CSV/ })).toBeVisible();
    await expect(menu.getByText('Filter row', { exact: true })).toBeVisible();
    await expect(menu).not.toContainText('⌘', { useInnerText: true });
    await page.keyboard.press('Escape');
    await expect(menu).toHaveCount(0);
    // Tables: "‹ Tables" pushes back to the list page (the tree); opening a
    // key from it pushes the workbench again.
    // The header ⋯ sits inside the window (it used to poke 7 px past the edge).
    await expectFullyInViewport(page, page.getByTestId('page-header').getByRole('button', { name: 'More actions' }), 'the header ⋯');
    await page.getByRole('button', { name: 'Back to Tables', exact: true }).tap();
    const tree = page.getByRole('tree', { name: 'Schema' });
    await expect(tree).toBeVisible();
    await expect(page.getByTestId('db-conn-state')).toBeHidden();
    // A table opens its structure: pushed back to the workbench.
    await tree.getByRole('button', { name: 'Expand shop' }).tap();
    await tree.getByRole('button', { name: 'Expand Tables' }).first().tap();
    await tree.locator('.node-label', { hasText: /^orders/ }).first().tap();
    await expect(page.getByTestId('db-conn-state')).toBeVisible();
    await expect(page.getByRole('button', { name: 'Back to Tables', exact: true })).toBeVisible();
    await assertFitsWidth(page);
  });

  test('phone: a connection that can\'t connect says so once, with Retry and Edit under the header', async ({ page }, info) => {
    test.skip(!/iphone|android/.test(info.project.name), 'phone projects');
    const { ctx, base } = await apiCtx();
    const name = `refused-phone-${Math.random().toString(36).slice(2, 7)}`;
    // Port 9 on loopback refuses at once: no host is contacted.
    const r = await ctx.post(`${base}/api/v1/workspaces/${workspaceId}/connections`, { data: { name, kind: 'redis', params: { host: '127.0.0.1', port: 9, database: '0' } } });
    expect(r.ok()).toBeTruthy();
    await ctx.dispose();
    await gotoPage(page);
    await row(page, name).click();
    await expect(page.getByTestId('db-conn-state')).toContainText('Can’t connect', { timeout: 30_000 });
    const banner = page.getByTestId('db-conn-error-banner');
    await expect(banner).toBeVisible();
    await expect(page.getByRole('button', { name: 'Retry', exact: true })).toHaveCount(1);
    await banner.getByRole('button', { name: 'Retry', exact: true }).click();
    await expect(page.getByTestId('db-conn-state')).toContainText('Can’t connect', { timeout: 30_000 });
    // Nothing to list while it can't connect: no Tables page, the error owns the page (☰ stays).
    await expect(page.getByRole('button', { name: 'Back to Tables', exact: true })).toHaveCount(0);
    await expectFullyInViewport(page, page.getByTestId('open-navigator'), 'the navigator button');
    await assertFitsWidth(page);
  });
});
