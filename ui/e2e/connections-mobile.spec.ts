import { test, expect, type Page, type APIRequestContext } from '@playwright/test';
import { apiCtx, seedWorkspace } from './seed';

// Unified Connections hub — PHONE/responsive usability guard. Runs on every
// device project (phones + tablets).
//
// `#/connections` now renders the DB-workbench page (the unified hub): its
// sidebar tree lists EVERY profile kind (ssh/custom + DB engines + Kafka
// clusters) in the shared section tree, with per-kind glyphs, a `kind` tag on
// each row, and type-filter chips. On a phone the page stacks into accordions;
// the Connections accordion holds the tree. These tests assert LAYOUT +
// reachability (rows render, tree fits the viewport width, tags/chips work) —
// the seeded hosts can't actually connect in CI.

let workspaceId = '';
let prodSectionId = '';

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
  await expect(page.locator('.db-page')).toBeVisible({ timeout: 30_000 });
  await expect(page.locator('.conn-row').first()).toBeVisible({ timeout: 20_000 });
}

/** The DOCUMENT must not scroll horizontally, and no element in the page may
 *  extend past the viewport (allow sub-pixel rounding). */
async function assertFitsWidth(page: Page): Promise<void> {
  const r = await page.evaluate(() => {
    const de = document.documentElement;
    let widest = 0;
    document.querySelectorAll<HTMLElement>('.db-page *').forEach((el) => {
      const rect = el.getBoundingClientRect();
      if (rect.right > widest) widest = rect.right;
    });
    return { docScrollW: de.scrollWidth, docClientW: de.clientWidth, vw: window.innerWidth, widest: Math.round(widest) };
  });
  expect(r.docScrollW).toBeLessThanOrEqual(r.docClientW + 1);
  expect(r.docClientW).toBeLessThanOrEqual(r.vw + 1);
  expect(r.widest).toBeLessThanOrEqual(r.vw + 2);
}

test.describe('connections hub — responsive', () => {
  test('unified tree renders every seeded profile and fits the viewport', async ({ page }) => {
    await gotoPage(page);

    await expect(page.locator('.conn-name', { hasText: 'prod-web-bastion' }).first()).toBeVisible();
    await expect(page.locator('.conn-name', { hasText: 'ungrouped-host' }).first()).toBeVisible();
    await expect(page.locator('.conn-name', { hasText: 'my-custom-cli' }).first()).toBeVisible();

    await assertFitsWidth(page);
  });

  test('section folders + Ungrouped group render', async ({ page }) => {
    await gotoPage(page);
    await expect(page.locator('.sec-name', { hasText: /Production servers/i }).first()).toBeVisible();
    await expect(page.locator('.sec-name', { hasText: /Empty staging folder/i }).first()).toBeVisible();
    await expect(page.locator('.sec-name', { hasText: /Ungrouped/i }).first()).toBeVisible();
    await assertFitsWidth(page);
  });

  test('rows carry per-kind tags; env badges still show', async ({ page }) => {
    await gotoPage(page);
    const bastion = page.locator('.conn-row', { hasText: 'prod-web-bastion' }).first();
    await expect(bastion.locator('.kind-tag')).toHaveText('ssh');
    await expect(bastion.locator('.env-badge')).toHaveText('PROD');
    const custom = page.locator('.conn-row', { hasText: 'my-custom-cli' }).first();
    await expect(custom.locator('.kind-tag')).toHaveText('custom');
    await assertFitsWidth(page);
  });

  test('type-filter chips narrow the tree and hide empty sections', async ({ page }) => {
    await gotoPage(page);
    await page.locator('[data-testid="connhub-filter-ssh"]').click();
    await expect(page.locator('.conn-name', { hasText: 'prod-web-bastion' }).first()).toBeVisible();
    await expect(page.locator('.conn-name', { hasText: 'my-custom-cli' })).toHaveCount(0);
    // Sections with no matching descendants disappear while filtering.
    await expect(page.locator('.sec-name', { hasText: /Empty staging folder/i })).toHaveCount(0);

    await page.locator('[data-testid="connhub-filter-custom"]').click();
    await expect(page.locator('.conn-name', { hasText: 'my-custom-cli' }).first()).toBeVisible();
    await expect(page.locator('.conn-name', { hasText: 'prod-web-bastion' })).toHaveCount(0);

    await page.locator('[data-testid="connhub-filter-all"]').click();
    await expect(page.locator('.conn-name', { hasText: 'prod-web-bastion' }).first()).toBeVisible();
    await expect(page.locator('.conn-name', { hasText: 'my-custom-cli' }).first()).toBeVisible();
    await assertFitsWidth(page);
  });

  test('tree search filters to a flat match list and clears back', async ({ page }) => {
    await gotoPage(page);
    await page.locator('.tree-search-input').fill('bastion');
    await expect(page.locator('.conn-name', { hasText: 'prod-web-bastion' }).first()).toBeVisible();
    await expect(page.locator('.conn-name', { hasText: 'my-custom-cli' })).toHaveCount(0);
    await assertFitsWidth(page);
    await page.locator('.tree-search-clear').click();
    await expect(page.locator('.conn-name', { hasText: 'my-custom-cli' }).first()).toBeVisible();
  });
});
