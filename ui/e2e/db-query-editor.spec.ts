import { test, expect, type Page } from '@playwright/test';
import { apiCtx, seedWorkspace, seedDockerConnection } from './seed';

// ─────────────────────────────────────────────────────────────────────────────
// DB Explorer — query-editor features (against the live Docker MySQL stack).
// Skips cleanly when the dev DB stack isn't up (like the db-sweep-* specs).
//
//   1. Run only the SELECTED statement (multi-statement buffer, run-selection).
//   2. Run the statement under the CURSOR when nothing is selected.
//   3. Format / beautify the SQL.
//   4. Query-level variables (:name) substituted before running.
//   5. SQL syntax highlighting is active (token spans rendered).
// ─────────────────────────────────────────────────────────────────────────────

let workspaceId = '';
let connId: string | null = null;
let connIdRedis: string | null = null;
const PHONE_MAX = 640;

test.beforeAll(async () => {
  test.setTimeout(120_000);
  const { ctx, base } = await apiCtx();
  workspaceId = await seedWorkspace(ctx, base);
  try {
    connId = await seedDockerConnection(ctx, base, workspaceId, 'mysql');
  } catch {
    connId = null;
  }
  try {
    connIdRedis = await seedDockerConnection(ctx, base, workspaceId, 'redis');
  } catch {
    connIdRedis = null;
  }
  await ctx.dispose().catch(() => {});
});

test.beforeEach(async ({ page }) => {
  await page.addInitScript((wsId) => {
    localStorage.setItem('otto_workspace', wsId as string);
    localStorage.setItem('otto_rail_expanded', '0');
  }, workspaceId);
});

function isPhone(page: Page): boolean {
  const w = page.viewportSize()?.width ?? 0;
  return w > 0 && w <= PHONE_MAX;
}

async function openConn(page: Page, name: string): Promise<void> {
  await page.goto('/#/database');
  await expect(page.locator('.shell')).toBeVisible({ timeout: 30_000 });
  const conn = page.locator('.conn-list .conn-name', { hasText: name });
  await expect(conn.first()).toBeVisible({ timeout: 30_000 });
  await conn.first().click();
  await expect(page.locator('.main-tabs')).toBeVisible({ timeout: 20_000 });
}
const openMysql = (page: Page) => openConn(page, 'e2e-mysql');

async function ensureEditorOpen(page: Page): Promise<void> {
  if (!isPhone(page)) return;
  const editor = page.locator('.qe-edit');
  if (!(await editor.isVisible().catch(() => false))) {
    await page.locator('.qe-acc-head', { hasText: 'Editor' }).click();
  }
  await expect(editor).toBeVisible();
}

async function ensureResultsOpen(page: Page): Promise<void> {
  if (!isPhone(page)) return;
  const results = page.locator('.qe-results');
  if (!(await results.isVisible().catch(() => false))) {
    await page.locator('.qe-acc-head', { hasText: 'Results' }).click();
  }
  await expect(results).toBeVisible();
}

/** Replace the editor content with `sql` (CodeMirror), retrying until it sticks. */
async function setEditor(page: Page, sql: string): Promise<void> {
  await ensureEditorOpen(page);
  const content = page.locator('.qe-edit .cm-content');
  const mod = process.platform === 'darwin' ? 'Meta' : 'Control';
  const want = sql.replace(/\s+/g, ' ').trim();
  for (let attempt = 0; attempt < 3; attempt++) {
    await content.click();
    await page.keyboard.press(`${mod}+A`);
    await page.keyboard.press('Delete');
    await content.pressSequentially(sql, { delay: 6 });
    await page.keyboard.press('Escape'); // dismiss any autocomplete popup
    const got = ((await content.textContent()) ?? '').replace(/\s+/g, ' ').trim();
    if (got.startsWith(want)) return;
  }
}

async function clickRun(page: Page): Promise<void> {
  await page.getByRole('button', { name: /^Run/ }).first().click();
  await expect(page.getByRole('button', { name: /^Run/ }).first()).toBeVisible({ timeout: 20_000 });
  await ensureResultsOpen(page);
}

test.describe('DB Explorer · query editor', () => {
  test.describe.configure({ mode: 'serial' });

  test('connection seeds & is reachable', () => {
    expect(connId, 'mysql not reachable at 127.0.0.1:13306').not.toBeNull();
  });

  test('runs only the SELECTED statement (and keeps the full buffer)', async ({ page }) => {
    test.skip(connId == null, 'mysql unavailable');
    await openMysql(page);
    await setEditor(page, "SELECT 'AAA' AS tag;\nSELECT 'BBB' AS tag;");
    // Select the second statement's line, then Run.
    await page.locator('.qe-edit .cm-line').nth(1).click({ clickCount: 3 });
    await clickRun(page);

    const body = page.locator('.grid tbody');
    await expect(body.getByText('BBB', { exact: true })).toBeVisible({ timeout: 15_000 });
    await expect(body.getByText('AAA', { exact: true })).toHaveCount(0);

    // The transient run must NOT have clobbered the editor — both statements remain.
    const text = ((await page.locator('.qe-edit .cm-content').textContent()) ?? '').replace(
      /\s+/g,
      ' ',
    );
    expect(text).toContain('AAA');
    expect(text).toContain('BBB');
  });

  test('runs the statement under the CURSOR when nothing is selected', async ({ page }) => {
    test.skip(connId == null, 'mysql unavailable');
    await openMysql(page);
    await setEditor(page, "SELECT 'AAA' AS tag;\nSELECT 'BBB' AS tag;");
    // Click into the FIRST line (cursor only, no selection), then Run.
    await page.locator('.qe-edit .cm-line').first().click();
    await clickRun(page);

    const bodyc = page.locator('.grid tbody');
    await expect(bodyc.getByText('AAA', { exact: true })).toBeVisible({ timeout: 15_000 });
    await expect(bodyc.getByText('BBB', { exact: true })).toHaveCount(0);
  });

  test('Format beautifies the SQL', async ({ page }) => {
    test.skip(connId == null, 'mysql unavailable');
    await openMysql(page);
    await setEditor(page, 'select 1 as a, 2 as b');
    await page.getByRole('button', { name: 'Format', exact: true }).click();
    // sql-formatter upper-cases keywords + adds newlines.
    await expect
      .poll(async () => ((await page.locator('.qe-edit .cm-content').textContent()) ?? ''))
      .toContain('SELECT');
  });

  test('string variable (default) is auto-quoted before running', async ({ page }) => {
    test.skip(connId == null, 'mysql unavailable');
    await openMysql(page);
    await setEditor(page, 'SELECT * FROM customers WHERE email = :em');
    // The variables bar appears for :em, defaulting to type=string.
    const bar = page.locator('.qe-vars');
    await expect(bar).toBeVisible({ timeout: 10_000 });
    await expect(bar.locator('.qe-var-name', { hasText: 'em' })).toBeVisible();
    await expect(bar.locator('.qe-var-type').first()).toHaveValue('string');
    // Type the RAW value (no quotes) — default string type auto-quotes it, so
    // `email = ada@example.com` becomes `email = 'ada@example.com'`.
    await bar.locator('.qe-var-input').first().fill('ada@example.com');
    await clickRun(page);
    await expect(page.locator('.grid tbody').getByText('ada@example.com').first()).toBeVisible({
      timeout: 15_000,
    });
  });

  test('number variable is substituted RAW (unquoted)', async ({ page }) => {
    test.skip(connId == null, 'mysql unavailable');
    await openMysql(page);
    // LIMIT requires an unquoted integer — `LIMIT '1'` is a syntax error, so this
    // only succeeds if the number type substitutes raw (not as a string literal).
    await setEditor(page, 'SELECT email FROM customers LIMIT :lim');
    const bar = page.locator('.qe-vars');
    await expect(bar).toBeVisible({ timeout: 10_000 });
    await bar.locator('.qe-var-type').first().selectOption('number');
    await bar.locator('.qe-var-input').first().fill('1');
    await clickRun(page);
    await expect(page.locator('.grid tbody tr:not(.spacer)')).toHaveCount(1, { timeout: 15_000 });
  });

  test('SQL syntax highlighting is active (token spans rendered)', async ({ page }) => {
    test.skip(connId == null, 'mysql unavailable');
    await openMysql(page);
    await setEditor(page, 'SELECT id, name FROM customers');
    // With the SQL language + highlight style active, CodeMirror wraps tokens in
    // styled <span>s — plain text would have none.
    await expect
      .poll(async () => page.locator('.qe-edit .cm-line span').count(), { timeout: 10_000 })
      .toBeGreaterThan(0);
  });

  test('redis: highlighting + Format hidden + no false {tag} variable', async ({ page }) => {
    test.skip(connIdRedis == null, 'redis unavailable');
    await openConn(page, 'e2e-redis-docker');
    // `{tag}` is a Cluster hash-tag, not a variable; `user:1` is a key namespace.
    await setEditor(page, 'GET {tag}:field\nSET user:1 x');
    // The built-in redis StreamLanguage colors tokens → at least one span.
    await expect
      .poll(async () => page.locator('.qe-edit .cm-line span').count(), { timeout: 10_000 })
      .toBeGreaterThan(0);
    // Format only applies to SQL — it must NOT be shown for redis.
    await expect(page.getByRole('button', { name: 'Format', exact: true })).toHaveCount(0);
    // Neither the hash-tag nor the `:` namespace may be mistaken for a variable.
    await expect(page.locator('.qe-vars')).toHaveCount(0);
  });

  test('smart completion: tables after FROM, PK column first in WHERE', async ({ page }) => {
    test.skip(connId == null, 'mysql unavailable');
    test.skip(isPhone(page), 'completion popup assertions target the desktop layout');
    await openMysql(page);
    await ensureEditorOpen(page);
    const content = page.locator('.qe-edit .cm-content');
    const pop = page.locator('.cm-tooltip-autocomplete');
    const mod = process.platform === 'darwin' ? 'Meta' : 'Control';
    const reset = async () => {
      await content.click();
      await page.keyboard.press(`${mod}+A`);
      await page.keyboard.press('Delete');
    };

    // (1) After `FROM ` the popup offers the seeded tables (above keywords).
    await reset();
    await content.pressSequentially('SELECT * FROM ', { delay: 10 });
    await page.keyboard.press('Control+Space');
    await expect(pop).toBeVisible({ timeout: 10_000 });
    await expect(
      pop.locator('.cm-completionLabel', { hasText: /^orders$/ }).first(),
    ).toBeVisible({ timeout: 10_000 });
    await expect(
      pop.locator('.cm-completionLabel', { hasText: /^customers$/ }).first(),
    ).toBeVisible();
    await page.keyboard.press('Escape');

    // (2) After `WHERE ` the popup offers columns, the PRIMARY KEY first
    //     (indexes first) — `orders.id` ranks above plain columns / keywords.
    await reset();
    await content.pressSequentially('SELECT * FROM orders WHERE ', { delay: 10 });
    await page.keyboard.press('Control+Space');
    await expect(pop).toBeVisible({ timeout: 10_000 });
    await expect(pop.locator('li').first().locator('.cm-completionLabel')).toHaveText('id', {
      timeout: 10_000,
    });
    await page.keyboard.press('Escape');
  });
});
