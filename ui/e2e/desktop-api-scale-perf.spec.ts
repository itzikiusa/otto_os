import { expect, test, type Page } from '@playwright/test';
import { apiCtx, seedWorkspace } from './seed';
import { openApiEditor, openPage } from './helpers';

// ─────────────────────────────────────────────────────────────────────────────
// API client at scale (perf sweep B: SB-01, SB-02, SB-05). Budgets are DOM /
// socket counts, not timings, so they hold on any machine:
//   • a 3,000-request workspace keeps the collections sidebar windowed — big
//     workspaces open with folders collapsed, and a search that matches hundreds of
//     requests still mounts only a viewport's worth of rows;
//   • the API client's scratch editors (body, scripts, docs, response) never
//     open a language-server socket (each one used to spawn a server process).
// Mounted production components; only the api-client LIST transport is mocked.
//
// Desktop-browser project only.
// ─────────────────────────────────────────────────────────────────────────────

let workspaceId = '';

test.beforeAll(async () => {
  const { ctx, base } = await apiCtx();
  try {
    workspaceId = await seedWorkspace(ctx, base);
  } finally {
    await ctx.dispose().catch(() => {});
  }
});

test.beforeEach(async ({ page }, testInfo) => {
  test.skip(testInfo.project.name !== 'desktop-browser', 'desktop-browser only');
  await page.addInitScript((wsId) => {
    localStorage.setItem('otto_workspace', wsId as string);
    localStorage.setItem('otto_rail_expanded', '0');
  }, workspaceId);
});

/** 3,000 requests spread over 30 collections × 10 folders. */
function bigTree(wid: string) {
  const now = '2026-09-01T00:00:00Z';
  const cols: Record<string, unknown>[] = [];
  const leaves: string[] = [];
  for (let r = 0; r < 30; r++) {
    const rid = `col-${r}`;
    cols.push({ id: rid, workspace_id: wid, name: `Service ${r}`, parent_id: null, position: r, created_at: now });
    for (let s = 0; s < 10; s++) {
      const sid = `col-${r}-${s}`;
      cols.push({ id: sid, workspace_id: wid, name: `Folder ${r}.${s}`, parent_id: rid, position: s, created_at: now });
      leaves.push(sid);
    }
  }
  const reqs = Array.from({ length: 3000 }, (_, i) => ({
    id: `req-${i}`, workspace_id: wid, collection_id: leaves[i % leaves.length],
    name: `${i % 3 === 0 ? 'Create' : 'Get'} resource ${i}`, method: i % 3 === 0 ? 'POST' : 'GET',
    url: `https://api.example.com/v1/resource/${i}`, headers: [], query: [], body_mode: 'none', body: '',
    auth: { type: 'none' }, extras: null, position: i, created_at: now, updated_at: now,
  }));
  return { cols, reqs };
}

async function mockBigWorkspace(page: Page): Promise<void> {
  const { cols, reqs } = bigTree(workspaceId);
  const base = `/api/v1/workspaces/${workspaceId}/api-client`;
  await page.route(`**${base}/collections`, (route) =>
    route.request().method() === 'GET' ? route.fulfill({ json: cols }) : route.fallback());
  await page.route(`**${base}/requests`, (route) =>
    route.request().method() === 'GET' ? route.fulfill({ json: reqs }) : route.fallback());
}

test('3,000 saved requests: the sidebar stays windowed while browsing and searching', async ({ page }) => {
  await mockBigWorkspace(page);
  await openPage(page, 'api');
  const tree = page.locator('.tree-wrap');
  const rows = tree.locator('.req-row');
  const heads = tree.locator('.col-head');

  // Big workspaces open collapsed: collection rows only, no request rows.
  await expect(heads.first()).toBeVisible({ timeout: 30_000 });
  await expect(rows).toHaveCount(0);
  expect(await heads.count()).toBeLessThanOrEqual(60);

  // Hundreds of matches force their branches open — still only a window is mounted.
  const input = page.getByLabel('Search collections and requests');
  await input.fill('resource 12');
  await expect(rows.first()).toBeVisible();
  expect(await rows.count()).toBeLessThanOrEqual(150);
  expect(await tree.locator('.req-row, .col-head').count()).toBeLessThanOrEqual(150);

  // A precise search finds a request deep in the list.
  await input.fill('resource 2999');
  await expect(rows).toHaveCount(1);
  await expect(tree.getByText('Get resource 2999', { exact: true })).toBeVisible();

  // Opening one folder shows only its requests.
  await input.fill('');
  await expect(rows).toHaveCount(0);
  const folder = (name: string) => tree.locator('.col-toggle', { has: page.locator('.col-name', { hasText: new RegExp(`^${name.replace('.', '\\.')}$`) }) });
  await folder('Service 0').click();
  await folder('Folder 0.0').click();
  await expect(rows.first()).toBeVisible();
  expect(await rows.count()).toBeLessThanOrEqual(150);
});

test('API client editors never open a language-server socket', async ({ page }) => {
  const lspSockets: string[] = [];
  page.on('websocket', (ws) => {
    if (ws.url().includes('/ws/lsp')) lspSockets.push(ws.url());
  });
  await openApiEditor(page);
  for (const name of [/^Body/, /^Scripts/, /^Docs/, /^Body/]) {
    const tab = page.getByRole('tab', { name });
    if (await tab.count()) await tab.first().click();
    await page.waitForTimeout(150);
  }
  // Give a (wrongly) attached server time to connect before asserting.
  await page.waitForTimeout(800);
  expect(lspSockets).toEqual([]);
});
