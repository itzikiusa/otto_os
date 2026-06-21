import { test, expect, type Page, type APIRequestContext } from '@playwright/test';
import { apiCtx, seedWorkspace } from './seed';

// Durable mobile/responsive guard for the Message Brokers page. Runs on every
// device project (phones + tablets). Seeds a workspace + a Kafka cluster via the
// API so the sidebar list, cluster header, and tab bar all render — the cluster
// can't actually CONNECT (no live Kafka in CI), so this asserts LAYOUT only:
// content fits the viewport width, sections are scrollable, and the cluster list
// + tab bar are reachable/usable. Live topic/message data is NOT exercised.

let workspaceId = '';

async function seedCluster(ctx: APIRequestContext, base: string, wsId: string): Promise<void> {
  const r = await ctx.post(`${base}/api/v1/workspaces/${wsId}/brokers/clusters`, {
    data: {
      name: 'mobile-test-kafka',
      bootstrap_servers: 'broker-1.example.com:9092,broker-2.example.com:9092',
      security_protocol: 'plaintext',
      environment: 'prod',
      read_only: false,
    },
  });
  if (!r.ok()) throw new Error(`seed cluster → ${r.status()} ${await r.text()}`);
}

test.beforeAll(async () => {
  const { ctx, base } = await apiCtx();
  workspaceId = await seedWorkspace(ctx, base);
  await seedCluster(ctx, base, workspaceId);
  await ctx.dispose();
});

test.beforeEach(async ({ page }) => {
  await page.addInitScript((wsId) => {
    localStorage.setItem('otto_workspace', wsId as string);
    localStorage.setItem('otto_rail_expanded', '0');
  }, workspaceId);
});

/** No element inside the page may extend past the viewport's right edge, and the
 *  document must not scroll horizontally. (Internal horizontal scrollers like the
 *  tab strip are exempt: we measure only that the PAGE itself fits.) */
async function assertFitsWidth(page: Page): Promise<void> {
  const r = await page.evaluate(() => {
    const de = document.documentElement;
    return { docScrollW: de.scrollWidth, docClientW: de.clientWidth, vw: window.innerWidth };
  });
  // Document must not be horizontally scrollable (allow 1px rounding).
  expect(r.docScrollW).toBeLessThanOrEqual(r.docClientW + 1);
  expect(r.docClientW).toBeLessThanOrEqual(r.vw + 1);
}

test('brokers page fits the viewport and is navigable', async ({ page }) => {
  await page.goto('/#/brokers');
  await expect(page.locator('.brokers-page')).toBeVisible({ timeout: 30_000 });

  // Cluster list is visible/usable.
  const row = page.locator('.cluster .cn', { hasText: 'mobile-test-kafka' }).first();
  await expect(row).toBeVisible({ timeout: 15_000 });
  await assertFitsWidth(page);

  // Select the cluster → header + tab bar render.
  await row.click();
  await expect(page.locator('.cluster-head .name')).toBeVisible({ timeout: 15_000 });
  // The tab bar and its tabs are present/usable.
  const tabs = page.locator('.tabs');
  await expect(tabs).toBeVisible();
  await expect(page.locator('.tabs button', { hasText: 'Overview' })).toBeVisible();
  await expect(page.locator('.tabs button', { hasText: 'Topics' })).toBeVisible();
  await assertFitsWidth(page);

  // Switching tabs works (Topics renders its own panel; data won't load without
  // a live broker, but the tab/panel must be reachable).
  await page.locator('.tabs button', { hasText: 'Topics' }).first().click();
  await expect(page.locator('.tab-body')).toBeVisible();
  await assertFitsWidth(page);
});

test('add-cluster form fits the viewport and is usable', async ({ page }) => {
  await page.goto('/#/brokers');
  await expect(page.locator('.brokers-page')).toBeVisible({ timeout: 30_000 });

  await page.locator('button[title="Add cluster"]').first().click({ force: true });
  const sheet = page.locator('.sheet');
  await expect(sheet).toBeVisible();
  // The modal sheet must not overflow the viewport (its footer buttons stay
  // reachable). Measure the modal backdrop's horizontal overflow.
  const fit = await page.evaluate(() => {
    const bd = document.querySelector('.backdrop') as HTMLElement | null;
    return bd ? { scrollW: bd.scrollWidth, clientW: bd.clientWidth } : null;
  });
  expect(fit).not.toBeNull();
  expect(fit!.scrollW).toBeLessThanOrEqual(fit!.clientW + 1);
  // Footer actions are visible and tappable. Scope to the modal sheet — the
  // sidebar also has an "Add cluster" icon button (title), which would otherwise
  // make the name match ambiguous.
  await expect(sheet.getByRole('button', { name: /^Add cluster$/ })).toBeVisible();
  const cancel = sheet.getByRole('button', { name: /^Cancel$/ });
  await expect(cancel).toBeVisible();
  await cancel.click();
  await expect(sheet).toBeHidden();
});

// Phone-only: the two sections (cluster list + cluster content) are independent
// collapsible regions. This behavior is gated behind the ≤640px layout, so it
// only runs on phone-width projects.
test('phone: sections collapse independently and scroll', async ({ page }) => {
  await page.goto('/#/brokers');
  await expect(page.locator('.brokers-page')).toBeVisible({ timeout: 30_000 });
  // Collapsible sections are gated behind the ≤640px layout — skip on
  // tablet/landscape widths where the desktop two-column layout applies.
  const w = await page.evaluate(() => window.innerWidth);
  test.skip(w > 640, `collapsible sections are a phone-width (≤640px) layout (vw=${w})`);

  const row = page.locator('.cluster .cn', { hasText: 'mobile-test-kafka' }).first();
  await expect(row).toBeVisible({ timeout: 15_000 });
  await row.click();
  await expect(page.locator('.cluster-head .name')).toBeVisible({ timeout: 15_000 });

  // Cluster-list region must be its own bounded, scrollable section.
  const listScroll = await page.evaluate(() => {
    const el = document.querySelector('.cluster-list') as HTMLElement | null;
    if (!el) return null;
    const c = getComputedStyle(el);
    return { overflowY: c.overflowY, maxHeight: c.maxHeight };
  });
  expect(listScroll).not.toBeNull();
  expect(['auto', 'scroll']).toContain(listScroll!.overflowY);

  // Selecting auto-collapses the list (caret toggle). Re-expand it via the
  // tappable section header, then collapse the content section.
  const clustersToggle = page.locator('.sec-toggle').first();
  await expect(clustersToggle).toBeVisible();
  await clustersToggle.click();
  await expect(page.locator('.cluster-list')).toBeVisible();

  const contentToggle = page.locator('.content-toggle').first();
  await expect(contentToggle).toBeVisible();
  // Collapse the content → its tab bar/body hide, header stays.
  await contentToggle.click();
  await expect(page.locator('.tab-body')).toBeHidden();
  await expect(page.locator('.cluster-head .name')).toBeVisible();
  // Expand again → tabs come back.
  await contentToggle.click();
  await expect(page.locator('.tabs')).toBeVisible();
});
