import { test, expect, type APIRequestContext, type Page } from '@playwright/test';
import { mkdtempSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { apiCtx } from './seed';
import { expectNoHorizontalOverflow } from './helpers';

// "Available in": an MCP server added in workspace A as "All workspaces" is in
// a workspace C created AFTERWARDS — its list and a new session's MCP servers
// there — and narrowing it to "Selected: A, B" takes it out of C. One server
// row, shared (otto_state::mcp_scope). Real daemon, real PATCHes, the
// otto-fake-claude agent reporting the servers its --mcp-config gave it.

const PROBE = fileURLToPath(new URL('./fixtures/mcp-probe.mjs', import.meta.url));
// Review screenshots (light + dark, per project) go here when it exists.
const SHOTS = process.env.OTTO_E2E_SHOTS ?? '';

let ctx: APIRequestContext;
let base = '';

test.describe.configure({ mode: 'serial' });

test.beforeAll(async () => {
  ({ ctx, base } = await apiCtx());
});
test.afterAll(async () => {
  await ctx?.dispose();
});

async function workspace(name: string): Promise<{ id: string; dir: string }> {
  const dir = mkdtempSync(join(tmpdir(), 'otto-e2e-mcp-scope-'));
  const r = await ctx.post(`${base}/api/v1/workspaces`, { data: { name, root_path: dir } });
  expect(r.ok(), await r.text()).toBeTruthy();
  return { id: (await r.json()).id as string, dir };
}

async function openServers(page: Page, wsId: string): Promise<void> {
  await page.goto('/#/mcp/servers');
  await page.evaluate((id) => localStorage.setItem('otto_workspace', id), wsId);
  await page.reload();
}

const row = (page: Page, name: string) => page.getByTestId('mcp-server-row').filter({ hasText: name });

async function shot(page: Page, name: string): Promise<void> {
  if (!SHOTS) return;
  for (const scheme of ['light', 'dark'] as const) {
    await page.emulateMedia({ colorScheme: scheme });
    await page.screenshot({ path: join(SHOTS, `${name}-${test.info().project.name}-${scheme}.png`) });
  }
  await page.emulateMedia({ colorScheme: 'light' });
}

/** The MCP servers a new Claude thread in `ws` reports (`system init`). */
async function sessionServers(page: Page, ws: { id: string; dir: string }, title: string): Promise<string[]> {
  const created = await ctx.post(`${base}/api/v1/workspaces/${ws.id}/runtime-sessions`, { data: { provider: 'claude', cwd: ws.dir, title } });
  expect(created.ok(), await created.text()).toBeTruthy();
  const id = (await created.json()).id as string;
  await page.goto(`/#/threads/${id}`);
  const thread = page.getByTestId(`runtime-thread-${id}`);
  await expect(thread).toBeVisible({ timeout: 30_000 });
  await thread.getByTestId('runtime-composer').fill('hello');
  await thread.getByRole('button', { name: 'Send message' }).click();
  await expect(thread.getByText('echo: hello')).toBeVisible({ timeout: 30_000 });
  // ⌘K › "MCP servers for this thread": what the agent has.
  const bar = page.getByTestId('floating-bar');
  await expect(async () => {
    await page.keyboard.press('Meta+k');
    await expect(bar.getByRole('combobox')).toBeVisible({ timeout: 1_000 });
  }).toPass({ timeout: 15_000 });
  await bar.getByRole('combobox').fill('MCP servers for this thread');
  await bar.getByRole('option').filter({ hasText: 'MCP servers for this thread' }).first().click();
  const sheet = page.getByTestId('runtime-mcp-servers');
  await expect(sheet.locator('[data-testid="runtime-mcp-server"][data-name="otto"]')).toBeVisible({ timeout: 30_000 });
  const names = await sheet.locator('[data-testid="runtime-mcp-server"]').evaluateAll((els) => els.map((e) => e.getAttribute('data-name') ?? ''));
  await ctx.post(`${base}/api/v1/runtime-sessions/${id}/close`).catch(() => {});
  return names;
}

test('an all-workspaces server reaches a workspace added later until it is narrowed', async ({ page }, info) => {
  const name = `shared-probe-${info.project.name}`;
  const a = await workspace(`Scope A ${info.project.name}`);
  const b = await workspace(`Scope B ${info.project.name}`);

  // Add it in A as "All workspaces": a wider scope asks first.
  await openServers(page, a.id);
  await page.getByRole('button', { name: 'Add server' }).first().click();
  const form = page.getByRole('dialog').filter({ hasText: 'Add MCP server' });
  await form.getByLabel('Name', { exact: true }).fill(name);
  await form.getByTestId('mcp-server-runs').selectOption('native');
  await form.getByPlaceholder('npx').fill(process.execPath);
  await form.locator('textarea').first().fill(PROBE);
  await form.getByLabel('Available in').selectOption('all');
  await expect(form).toContainText('Also every workspace added later.');
  await form.getByRole('checkbox', { name: /Enable now/ }).check();
  await shot(page, 'add-form-all');
  await form.getByRole('button', { name: 'Add server', exact: true }).click();
  const confirm = page.getByRole('dialog').filter({ hasText: 'Share MCP server' });
  await expect(confirm).toContainText('every workspace, including ones added later');
  await expect(confirm).toContainText('never its credentials or settings');
  await confirm.getByRole('button', { name: 'Share', exact: true }).click();
  await expect(row(page, name).getByTestId('mcp-server-scope')).toHaveText('All workspaces', { timeout: 30_000 });

  // C is created AFTER the server: it has it, from A.
  const c = await workspace(`Scope C ${info.project.name}`);
  await openServers(page, c.id);
  await expect(row(page, name).getByTestId('mcp-server-scope')).toHaveText(`Shared from Scope A ${info.project.name}`, { timeout: 30_000 });
  await expectNoHorizontalOverflow(page);
  await shot(page, 'list-shared-from');
  if (info.project.name === 'desktop-browser') {
    expect(await sessionServers(page, c, 'Scope C before')).toContain(name);
    await openServers(page, c.id);
  }

  // Narrow to "Selected: A, B" (from C — the owner may): gone from C.
  await row(page, name).getByRole('button', { name: `More actions for ${name}` }).click();
  await page.getByRole('menuitem', { name: 'Available in…' }).click();
  const scope = page.getByRole('dialog').filter({ hasText: 'is available' });
  await scope.getByLabel('Available in').selectOption('selected');
  await scope.getByRole('checkbox', { name: `Scope B ${info.project.name}` }).check();
  await shot(page, 'scope-dialog-selected');
  await scope.getByTestId('mcp-scope-save').click();
  await expect(row(page, name)).toHaveCount(0, { timeout: 30_000 });
  if (info.project.name === 'desktop-browser') {
    expect(await sessionServers(page, c, 'Scope C after')).not.toContain(name);
  }
  // …and still in B, shared from A.
  await openServers(page, b.id);
  await expect(row(page, name).getByTestId('mcp-server-scope')).toHaveText(`Shared from Scope A ${info.project.name}`, { timeout: 30_000 });
});
