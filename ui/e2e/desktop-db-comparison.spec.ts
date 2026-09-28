import { test, expect, type Page } from '@playwright/test';
import { apiCtx, seedWorkspace } from './seed';
import { mockDbRoutes, seedMockDbConnection } from './db-mock';
import { expectNoHorizontalOverflow } from './helpers';

const suffix = Math.random().toString(36).slice(2, 8);
const EAST = `comparison-east-${suffix}`;
const WEST = `comparison-west-${suffix}`;
const EAST_SQL = 'SELECT id, name FROM east_customers';
const WEST_SQL = 'SELECT id, name FROM west_customers';
const EAST_DRAFT = 'SELECT id, name FROM east_customers WHERE unfinished_east';
const WEST_DRAFT = 'SELECT id, name FROM west_customers WHERE unfinished_west';
let workspaceId = '', eastId = '', westId = '';
type QueryCall = { connection: string; statement: string };
const derivedWarnings = new WeakMap<Page, string[]>();

// Real production-tagged profiles on the throwaway daemon; only engine calls
// are mocked. Both point at loopback port 9, never at a production database.
test.beforeAll(async () => {
  const { ctx, base } = await apiCtx();
  workspaceId = await seedWorkspace(ctx, base);
  eastId = await seedMockDbConnection(ctx, base, workspaceId, EAST);
  westId = await seedMockDbConnection(ctx, base, workspaceId, WEST);
  for (const [id, name] of [[eastId, EAST], [westId, WEST]]) {
    const response = await ctx.patch(`${base}/api/v1/connections/${id}`, { data: { name, kind: 'mysql', params: { host: '127.0.0.1', port: 9, user: 'mock', db: 'shop' }, environment: 'prod', read_only: true } });
    expect(response.ok(), await response.text()).toBeTruthy();
  }
  await ctx.dispose();
});

test.beforeEach(async ({ page }, info) => {
  test.skip(info.project.name !== 'desktop-browser', 'Desktop suite exercises narrow layout explicitly');
  await page.setViewportSize({ width: 1600, height: 1000 });
  const warnings: string[] = [];
  derivedWarnings.set(page, warnings);
  page.on('console', (message) => {
    if (message.text().includes('derived_inert')) warnings.push(message.text());
  });
  await page.addInitScript((id) => {
    localStorage.setItem('otto_workspace', id);
    localStorage.setItem('otto_rail_expanded', '0');
    localStorage.removeItem(`otto_db_open:${id}`);
  }, workspaceId);
});

test.afterEach(async ({ page }) => {
  expect(derivedWarnings.get(page) ?? [], 'No callback reads a destroyed query editor').toEqual([]);
});

async function fixture(page: Page): Promise<{ calls: QueryCall[]; revoke: () => Promise<void> }> {
  const calls: QueryCall[] = [];
  let eastAllowed = true;
  await page.route('**/api/v1/access/connection/*/capabilities*', (route) => {
    const parts = new URL(route.request().url()).pathname.split('/');
    const id = parts.at(-2)!;
    const operations = Object.fromEntries(['discover', 'db_browse', 'db_query', 'db_export'].map((operation) => [operation, {
      allowed: operation !== 'db_query' || id !== eastId || eastAllowed,
      reason: 'Comparison fixture', matched_rule_ids: [], mode: 'enforced',
    }]));
    return route.fulfill({ json: { kind: 'connection', resource_id: id, user_id: 'root', child: null, mode: 'enforced', operations } });
  });
  for (const [id, marker] of [[eastId, 'East customer'], [westId, 'West customer']]) {
    await mockDbRoutes(page, id);
    await page.route(`**/connections/${id}/db/object`, (route) => route.fulfill({ json: {
      name: id === eastId ? 'east_customers' : 'west_customers', kind: 'table',
      columns: [{ name: 'id', data_type: 'int', nullable: false, key: 'PRI', default: null, extra: null }, { name: 'name', data_type: 'varchar', nullable: false, key: null, default: null, extra: null }],
      primary_key: ['id'], indexes: [], foreign_keys: [], ddl: '', row_count: 2,
    } }));
    await page.route(`**/connections/${id}/db/query`, (route) => {
      const body = route.request().postDataJSON();
      calls.push({ connection: id, statement: body.statement });
      return route.fulfill({ json: {
        columns: [{ name: 'id', type_hint: 'INT' }, { name: 'name', type_hint: 'VARCHAR' }],
        rows: [[1, `${marker} one`], [2, `${marker} two`]],
        stats: { duration_ms: 3, row_count: 2 }, truncated: false,
      } });
    });
  }
  return { calls, revoke: async () => {
    eastAllowed = false;
    await page.evaluate(async (id) => {
      const path = '/src/lib/stores/resource-access.svelte.ts';
      const { resourceAccess } = await import(path);
      await resourceAccess.refresh({ kind: 'connection', resource_id: id });
    }, eastId);
  } };
}

async function openConnection(page: Page, name: string): Promise<void> {
  const picker = page.locator('.side-switch .ss', { hasText: 'Connections' }).first();
  if (await picker.isVisible()) await picker.click();
  await page.locator('.conn-list .conn-name', { hasText: name }).first().click();
  await expect(page.locator('.conn-tab.active .conn-tab-name')).toHaveText(name);
  await expect(page.locator('.query-editor')).toBeVisible();
}
async function draft(page: Page, statement: string): Promise<void> {
  const editor = page.locator('.query-editor .qe-edit .cm-content');
  await editor.click();
  await page.keyboard.press('ControlOrMeta+A');
  await page.keyboard.insertText(statement);
  await expect(editor).toHaveText(statement);
}
async function run(page: Page, statement: string, marker: string): Promise<void> {
  await draft(page, statement);
  await page.locator('.query-editor').locator('.btn.small.primary').filter({ hasText: 'Run' }).first().click();
  await expect(page.locator('.query-editor .grid')).toContainText(marker);
}
async function compareTwo(page: Page): Promise<void> {
  await page.goto('/#/database');
  await expect(page.locator('.shell')).toBeVisible({ timeout: 30_000 });
  await openConnection(page, EAST);
  await run(page, EAST_SQL, 'East customer one');
  await draft(page, EAST_DRAFT);
  await page.getByRole('button', { name: 'Compare results', exact: true }).click();
  await expect(page.getByTestId('connection-comparison')).toContainText('East customer one');
  await openConnection(page, WEST);
  await run(page, WEST_SQL, 'West customer one');
  await draft(page, WEST_DRAFT);
}

for (const scheme of ['light', 'dark'] as const) {
  test(`two production results remain live beside retained drafts (${scheme})`, async ({ page }, info) => {
    await page.addInitScript((value) => localStorage.setItem('otto_scheme', value), scheme);
    const { calls } = await fixture(page);
    await compareTwo(page);
    const comparison = page.getByTestId('connection-comparison');
    await expect(comparison).toContainText(EAST);
    await expect(comparison.locator('[data-env="prod"]')).toBeVisible();
    await expect(comparison.getByTestId('comparison-statement')).toHaveText(EAST_SQL);
    await expect(page.locator('.query-editor .cm-content')).toHaveText(WEST_DRAFT);
    await expect(page.locator('.query-editor .grid')).toContainText('West customer one');
    await expect(comparison.locator('.sel-del')).toHaveCount(0);
    await expect(comparison.getByRole('button', { name: /Apply|Run|Delete|Insert/i })).toHaveCount(0);
    await expectNoHorizontalOverflow(page);
    const screenshot = info.outputPath(`comparison-${scheme}.png`);
    await page.screenshot({ path: screenshot, fullPage: true });
    await info.attach(`comparison-${scheme}`, { path: screenshot, contentType: 'image/png' });

    const selector = comparison.getByLabel('Comparison source');
    const westOption = selector.locator('option').filter({ hasText: WEST });
    await selector.selectOption(await westOption.getAttribute('value') ?? '');
    await expect(comparison).toContainText('West customer one');
    const eastOption = selector.locator('option').filter({ hasText: EAST });
    await selector.selectOption(await eastOption.getAttribute('value') ?? '');
    await comparison.getByRole('button', { name: 'Focus source', exact: true }).click();
    await expect(page.locator('.conn-tab.active .conn-tab-name')).toHaveText(EAST);
    await expect(page.locator('.query-editor .cm-content')).toHaveText(EAST_DRAFT);
    await comparison.getByRole('button', { name: 'Clear comparison', exact: true }).click();
    await expect(comparison).toHaveCount(0);
    expect(calls).toEqual([
      { connection: eastId, statement: EAST_SQL },
      { connection: westId, statement: WEST_SQL },
    ]);
  });
}

test('source closure and permission revocation remove the comparison and any cell viewer', async ({ page }) => {
  const { calls, revoke } = await fixture(page);
  await compareTwo(page);
  const comparison = page.getByTestId('connection-comparison');
  await comparison.locator('.grid td[data-c="1"]').first().click({ button: 'right' });
  await page.getByRole('menuitem', { name: 'Expand value', exact: true }).click();
  await expect(page.getByRole('dialog', { name: 'Cell value' })).toBeVisible();
  await revoke();
  await expect(comparison).toHaveCount(0);
  await expect(page.getByRole('dialog', { name: 'Cell value' })).toHaveCount(0);
  await expect(page.locator('.query-editor .cm-content')).toHaveText(WEST_DRAFT);
  await page.getByRole('button', { name: 'Compare results', exact: true }).click();
  await expect(comparison).toContainText('West customer one');
  await page.locator('.conn-tab').filter({ hasText: WEST }).getByRole('button', { name: 'Close connection tab' }).click();
  await expect(comparison).toHaveCount(0);
  expect(calls).toHaveLength(2);
});

test('narrow workbench stacks the live comparison without page overflow or rerunning', async ({ page }, info) => {
  const { calls } = await fixture(page);
  await compareTwo(page);
  await page.setViewportSize({ width: 390, height: 844 });
  const comparison = page.getByTestId('connection-comparison');
  await expect(comparison).toBeVisible();
  await expect(comparison).toContainText('East customer one');
  await expect(page.locator('.query-editor .cm-content')).toHaveText(WEST_DRAFT);
  const editorBox = await page.locator('.query-editor').boundingBox();
  const comparisonBox = await comparison.boundingBox();
  expect(comparisonBox!.y).toBeGreaterThanOrEqual(editorBox!.y + editorBox!.height);
  const resultBox = await page.locator('.query-editor .qe-results').boundingBox();
  expect(comparisonBox!.y).toBeGreaterThanOrEqual(resultBox!.y + resultBox!.height);
  await expectNoHorizontalOverflow(page);
  await comparison.scrollIntoViewIfNeeded();
  await expect(comparison).toBeInViewport();
  await expect(comparison.getByLabel('Comparison source')).toBeInViewport();
  await expect(comparison.getByRole('button', { name: 'Focus source', exact: true })).toBeInViewport();
  const screenshot = info.outputPath('comparison-phone.png');
  await page.screenshot({ path: screenshot, fullPage: true });
  await info.attach('comparison-phone', { path: screenshot, contentType: 'image/png' });
  expect(calls).toHaveLength(2);
});


test('revocation dismisses the comparison cell menu before it can copy revoked data', async ({ page }) => {
  const { revoke } = await fixture(page);
  await compareTwo(page);
  await page.getByTestId('connection-comparison').locator('.grid td[data-c="1"]').first().click({ button: 'right' });
  await expect(page.getByRole('menuitem', { name: 'Copy value', exact: true })).toBeVisible();
  await revoke();
  await expect(page.getByTestId('connection-comparison')).toHaveCount(0);
  await expect(page.getByRole('menuitem', { name: 'Copy value', exact: true })).toHaveCount(0);
});

test('the pin survives switching to a connection saved on Structure', async ({ page }) => {
  const { calls } = await fixture(page);
  await compareTwo(page);
  await page.getByRole('tab', { name: 'Structure', exact: true }).click();
  await page.locator('.conn-tab').filter({ hasText: EAST }).locator('.conn-tab-name').click();
  await expect(page.getByTestId('connection-comparison')).toContainText('East customer one');
  await page.locator('.conn-tab').filter({ hasText: WEST }).locator('.conn-tab-name').click();
  await expect(page.getByRole('tab', { name: 'Structure', exact: true })).toHaveAttribute('aria-selected', 'true');
  await page.getByRole('tab', { name: 'Query', exact: true }).click();
  await expect(page.getByTestId('connection-comparison')).toContainText('East customer one');
  await expect(page.locator('.query-editor .cm-content')).toHaveText(WEST_DRAFT);
  expect(calls).toHaveLength(2);
});

test('the registered comparison command opens Query from Structure', async ({ page }) => {
  const { calls } = await fixture(page);
  await compareTwo(page);
  await page.getByRole('button', { name: 'Clear comparison', exact: true }).click();
  await page.getByRole('tab', { name: 'Structure', exact: true }).click();
  await page.evaluate(async () => {
    const path = '/src/lib/commands.svelte.ts';
    const { registry } = await import(path);
    const command = registry.all.find((entry: { id: string }) => entry.id === 'db.compare-results');
    if (!command) throw new Error('Comparison command was not registered');
    await command.run();
  });
  await expect(page.getByRole('tab', { name: 'Query', exact: true })).toHaveAttribute('aria-selected', 'true');
  await expect(page.getByTestId('connection-comparison')).toContainText('West customer one');
  expect(calls).toHaveLength(2);
});

test('a source run completing while parked updates the comparison', async ({ page }) => {
  const { calls } = await fixture(page);
  await compareTwo(page);
  const comparison = page.getByTestId('connection-comparison');
  await comparison.getByRole('button', { name: 'Focus source', exact: true }).click();
  const updatedSql = 'SELECT id, name FROM east_customers WHERE refreshed = 1';
  let finish: (() => void) | undefined;
  const responseReady = new Promise<void>((resolve) => { finish = resolve; });
  let requested = false;
  await page.route(`**/connections/${eastId}/db/query`, async (route) => {
    calls.push({ connection: eastId, statement: route.request().postDataJSON().statement });
    requested = true;
    await responseReady;
    await route.fulfill({ json: { columns: [{ name: 'id', type_hint: 'INT' }, { name: 'name', type_hint: 'VARCHAR' }], rows: [[3, 'East refreshed while parked']], stats: { duration_ms: 3, row_count: 1 }, truncated: false } });
  });
  try {
    await draft(page, updatedSql);
    await page.locator('.query-editor .btn.small.primary').filter({ hasText: 'Run' }).first().click();
    await expect.poll(() => requested).toBe(true);
    await page.locator('.conn-tab').filter({ hasText: WEST }).locator('.conn-tab-name').click();
    await expect(page.locator('.query-editor .cm-content')).toHaveText(WEST_DRAFT);
    finish!();
    await expect(comparison).toContainText('East refreshed while parked');
    await expect(comparison.getByTestId('comparison-statement')).toHaveText(updatedSql);
    await expect(page.locator('.query-editor .grid')).toContainText('West customer one');
    expect(calls.map(({ statement }) => statement)).toEqual([EAST_SQL, WEST_SQL, updatedSql]);
  } finally { finish!(); }
});
