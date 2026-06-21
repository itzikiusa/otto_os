import { test, expect, type Page, type APIRequestContext } from '@playwright/test';
import { apiCtx, seedWorkspace } from './seed';

// Connections page — PHONE/responsive usability guard. Runs on every device
// project (phones + tablets).
//
// The desktop connection row crams the name + host + status chips + SEVEN action
// controls (Open ▾, split, Test, SFTP, Edit, Pin, Delete) onto one fixed-height
// flex line. On a phone (≤640px) that overflowed the viewport horizontally — the
// actions ran off the right edge unreachable, and the whole document scrolled
// sideways. The mobile layout turns each connection into a CARD: an identity line
// (icon + name + badges + host) over a horizontally-scrollable action strip, so
// every control is reachable and the page only scrolls vertically. The header
// actions wrap to full width and the section-head icons grow to tap size.
//
// We seed connections directly via the API. NOTE: the seed.ts `seedRedis` helper
// registers a `redis` connection, but this page intentionally hides the four DB
// engines (mysql/redis/mongodb/clickhouse) — those live in the Database module —
// so we seed SSH + custom connections, which is what this page actually lists.
//
// The seeded host can't actually CONNECT (no live SSH in CI); these assert LAYOUT
// + reachability only (list renders/scrolls, cards fit width, action strip is
// reachable, the form fits + is usable, a new connection appears).

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

// Activate the seeded workspace (so connections load) and close the nav drawer
// (which defaults open on a fresh phone profile and would cover the page).
test.beforeEach(async ({ page }) => {
  await page.addInitScript((wsId) => {
    localStorage.setItem('otto_workspace', wsId as string);
    localStorage.setItem('otto_rail_expanded', '0');
  }, workspaceId);
});

async function gotoPage(page: Page): Promise<void> {
  await page.goto('/#/connections');
  await expect(page.locator('.page')).toBeVisible({ timeout: 30_000 });
  await expect(page.locator('.conn-row').first()).toBeVisible({ timeout: 20_000 });
}

/** The DOCUMENT must not scroll horizontally, and no element OTHER than the
 *  intentional `.conn-actions` horizontal scroller may extend past the viewport.
 *  (The action strip is an internal scroller, like a tab bar — it's exempt.) */
async function assertFitsWidth(page: Page): Promise<void> {
  const r = await page.evaluate(() => {
    const de = document.documentElement;
    let widest = 0;
    document.querySelectorAll<HTMLElement>('.page *').forEach((el) => {
      if (el.closest('.conn-actions')) return;
      const rect = el.getBoundingClientRect();
      if (rect.right > widest) widest = rect.right;
    });
    return { docScrollW: de.scrollWidth, docClientW: de.clientWidth, vw: window.innerWidth, widest: Math.round(widest) };
  });
  // The page itself must never scroll sideways (allow 1px rounding).
  expect(r.docScrollW).toBeLessThanOrEqual(r.docClientW + 1);
  expect(r.docClientW).toBeLessThanOrEqual(r.vw + 1);
  // No (non-exempt) element juts past the viewport edge (allow 2px sub-pixel).
  expect(r.widest).toBeLessThanOrEqual(r.vw + 2);
}

test.describe('connections page — responsive', () => {
  test('list renders, fits the viewport width, and scrolls vertically', async ({ page }) => {
    await gotoPage(page);

    // Seeded connections + their host descriptions are present.
    await expect(page.locator('.conn-name', { hasText: 'prod-web-bastion' }).first()).toBeVisible();
    await expect(page.locator('.conn-name', { hasText: 'ungrouped-host' }).first()).toBeVisible();
    await expect(page.locator('.conn-name', { hasText: 'my-custom-cli' }).first()).toBeVisible();

    await assertFitsWidth(page);

    // The page is the vertical scroll container; with several cards + sections it
    // overflows the viewport height and scrolls.
    const scroll = await page.locator('.page').evaluate((el) => ({
      clientH: el.clientHeight,
      scrollH: el.scrollHeight,
    }));
    expect(scroll.scrollH).toBeGreaterThan(scroll.clientH - 1);
  });

  test('section folders + Ungrouped group render', async ({ page }) => {
    await gotoPage(page);
    await expect(page.locator('.section-name', { hasText: /Production servers/i }).first()).toBeVisible();
    await expect(page.locator('.section-name', { hasText: /Empty staging folder/i }).first()).toBeVisible();
    await expect(page.locator('.section-name', { hasText: /Ungrouped/i }).first()).toBeVisible();
    await assertFitsWidth(page);
  });

  test('every row action is reachable (Open + Test + Edit etc.)', async ({ page }) => {
    await gotoPage(page);
    const row = page.locator('.conn-row', { hasText: 'prod-web-bastion' }).first();
    await expect(row).toBeVisible();

    // The primary Open control and the per-row Test/Edit/Delete buttons exist and
    // are within the row (reachable — on a phone via the scrollable action strip).
    await expect(row.getByRole('button', { name: /Open$/ }).first()).toBeVisible();
    await expect(row.getByRole('button', { name: /^Test$/ })).toBeAttached();
    await expect(row.locator('.icon-btn[title="Edit"]')).toBeAttached();
    await expect(row.locator('.icon-btn[title="Delete"]')).toBeAttached();
    // SSH rows expose the SFTP browse button.
    await expect(row.locator('.icon-btn[title="Browse files (SFTP)"]')).toBeAttached();
  });

  test('search filters the connection list', async ({ page }) => {
    await gotoPage(page);
    await page.locator('.conn-search-input').fill('bastion');
    // Flat results: only the matching connection remains.
    await expect(page.locator('.conn-name', { hasText: 'prod-web-bastion' }).first()).toBeVisible();
    await expect(page.locator('.conn-name', { hasText: 'my-custom-cli' })).toHaveCount(0);
    await assertFitsWidth(page);
    // Clearing returns to the tree.
    await page.locator('.conn-search-clear').click();
    await expect(page.locator('.conn-name', { hasText: 'my-custom-cli' }).first()).toBeVisible();
  });

  test('New Connection form fits the viewport and is usable', async ({ page }) => {
    await gotoPage(page);
    await page.getByRole('button', { name: /New Connection/ }).first().click();

    const sheet = page.locator('.sheet');
    await expect(sheet).toBeVisible();

    // The modal must not overflow the viewport horizontally (footer stays
    // reachable). Measure the backdrop's horizontal overflow.
    const fit = await page.evaluate(() => {
      const bd = document.querySelector('.backdrop') as HTMLElement | null;
      return bd ? { scrollW: bd.scrollWidth, clientW: bd.clientWidth } : null;
    });
    expect(fit).not.toBeNull();
    expect(fit!.scrollW).toBeLessThanOrEqual(fit!.clientW + 1);

    // Key fields are present + usable.
    const name = sheet.locator('#cf-name');
    await expect(name).toBeVisible();
    await name.fill('mobile-new-ssh');
    await expect(sheet.locator('.kind-chip', { hasText: 'ssh' })).toBeVisible();
    await expect(sheet.locator('#cf-host')).toBeVisible();

    // Footer actions are visible + tappable.
    await expect(sheet.getByRole('button', { name: /Create Connection/ })).toBeVisible();
    const cancel = sheet.getByRole('button', { name: /^Cancel$/ });
    await expect(cancel).toBeVisible();
    await cancel.click();
    await expect(sheet).toBeHidden();
  });

  test('creating a connection through the form makes it appear in the list', async ({ page }) => {
    await gotoPage(page);
    await page.getByRole('button', { name: /New Connection/ }).first().click();
    const sheet = page.locator('.sheet');
    await expect(sheet).toBeVisible();

    const unique = `e2e-mobile-${Date.now()}`;
    await sheet.locator('#cf-name').fill(unique);
    await sheet.locator('.kind-chip', { hasText: 'ssh' }).click();
    await sheet.locator('#cf-host').fill('new.example.com');
    await sheet.getByRole('button', { name: /Create Connection/ }).click();

    await expect(sheet).toBeHidden({ timeout: 15_000 });
    // The new connection shows up as a row.
    await expect(page.locator('.conn-name', { hasText: unique }).first()).toBeVisible({ timeout: 10_000 });
    await assertFitsWidth(page);
  });

  test('the form fits at every field-heavy state (custom kind)', async ({ page }) => {
    await gotoPage(page);
    await page.getByRole('button', { name: /New Connection/ }).first().click();
    const sheet = page.locator('.sheet');
    await expect(sheet).toBeVisible();

    // Switch to custom kind (different field set) — modal must still fit.
    await sheet.locator('.kind-chip', { hasText: 'custom' }).click();
    await expect(sheet.locator('#cf-template')).toBeVisible();
    const fit = await page.evaluate(() => {
      const bd = document.querySelector('.backdrop') as HTMLElement | null;
      return bd ? { scrollW: bd.scrollWidth, clientW: bd.clientWidth } : null;
    });
    expect(fit).not.toBeNull();
    expect(fit!.scrollW).toBeLessThanOrEqual(fit!.clientW + 1);
    await sheet.getByRole('button', { name: /^Cancel$/ }).click();
  });
});

test.describe('connections page — phone-only card layout (≤640px)', () => {
  test.skip(({ viewport }) => (viewport?.width ?? 0) > 640, 'phone-width card layout');

  test('rows stack into cards with a scrollable action strip', async ({ page }) => {
    await gotoPage(page);
    const card = page.locator('.conn-row', { hasText: 'prod-web-bastion' }).first();
    await expect(card).toBeVisible();

    // The card itself fits within the viewport width.
    const info = await card.evaluate((el) => {
      const r = el.getBoundingClientRect();
      const actions = el.querySelector('.conn-actions') as HTMLElement | null;
      const cs = actions ? getComputedStyle(actions) : null;
      return {
        right: Math.round(r.right),
        vw: window.innerWidth,
        flexDir: getComputedStyle(el).flexDirection,
        actOverflowX: cs?.overflowX ?? '',
        actScrollW: actions?.scrollWidth ?? 0,
        actClientW: actions?.clientWidth ?? 0,
      };
    });
    // Card stacks vertically (identity line over actions) and stays in-bounds.
    expect(info.flexDir).toBe('column');
    expect(info.right).toBeLessThanOrEqual(info.vw + 2);
    // Actions are an internal horizontal scroller (so all 5–7 controls are
    // reachable on the narrowest phones rather than overflowing the page).
    expect(['auto', 'scroll']).toContain(info.actOverflowX);
    expect(info.actScrollW).toBeGreaterThan(info.actClientW - 1);
  });

  test('header actions are full-width and do not overflow', async ({ page }) => {
    await gotoPage(page);
    const ha = page.locator('.header-actions');
    await expect(ha).toBeVisible();
    const info = await ha.evaluate((el) => {
      const r = el.getBoundingClientRect();
      return { right: Math.round(r.right), flexDir: getComputedStyle(el).flexDirection, vw: window.innerWidth };
    });
    expect(info.right).toBeLessThanOrEqual(info.vw + 2);
    // Header stacks: the page-header column-flexes so the action buttons drop to
    // their own full-width line.
    const headerDir = await page
      .locator('.page-header')
      .evaluate((el) => getComputedStyle(el).flexDirection);
    expect(headerDir).toBe('column');
  });

  test('connection name text is legibly sized (≥14px)', async ({ page }) => {
    await gotoPage(page);
    const size = await page
      .locator('.conn-name')
      .first()
      .evaluate((el) => parseFloat(getComputedStyle(el).fontSize));
    expect(size).toBeGreaterThanOrEqual(14);
  });
});
