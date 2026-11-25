import { test, expect, type APIRequestContext, type Page } from '@playwright/test';
import { mkdtempSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { apiCtx } from './seed';
import { expectNoHorizontalOverflow } from './helpers';

// "Available in": a governed MCP server added in workspace A as "All
// workspaces" is in a workspace C created AFTERWARDS — its list and the tools
// a new session there gets (the gateway list, "What my sessions see") — and
// narrowing it to "Selected: A, B" takes it out of C. A direct (native)
// server is never shared: the control says so and the API refuses. One
// server row, shared (otto_state::mcp_scope). Real daemon, real PATCHes.

const PROBE = fileURLToPath(new URL('./fixtures/mcp-probe.mjs', import.meta.url));
// Review screenshots (light + dark, per project) go here when it is set.
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

async function open(page: Page, wsId: string, route = 'mcp/servers'): Promise<void> {
  await page.goto(`/#/${route}`);
  await page.evaluate((id) => localStorage.setItem('otto_workspace', id), wsId);
  await page.reload();
}

const row = (page: Page, name: string) => page.getByTestId('mcp-server-row').filter({ hasText: name });

async function shot(page: Page, name: string): Promise<void> {
  if (!SHOTS) return;
  for (const scheme of ['light', 'dark'] as const) {
    await page.emulateMedia({ colorScheme: scheme });
    await page.waitForFunction((s) => document.documentElement.dataset.scheme === s, scheme, { timeout: 5_000 }).catch(() => {});
    await page.screenshot({ path: join(SHOTS, `r2-${name}-${test.info().project.name}-${scheme}.png`), animations: 'disabled' });
  }
  await page.emulateMedia({ colorScheme: 'light' });
}

/** The governed tools a new session in the current workspace gets. */
async function sessionTools(page: Page, wsId: string): Promise<string> {
  await open(page, wsId, 'mcp/otto');
  const panel = page.getByTestId('mcp-sessions-panel');
  await expect(panel).toContainText('Gateway tools from governed external servers', { timeout: 30_000 });
  return (await panel.textContent()) ?? '';
}

test('an all-workspaces server reaches a workspace added later until it is narrowed', async ({ page }, info) => {
  const name = `shared-probe-${info.project.name}`;
  const a = await workspace(`Scope A ${info.project.name}`);
  const b = await workspace(`Scope B ${info.project.name}`);

  // Add it in A as "All workspaces": a wider scope asks first, naming what
  // is shared and who can use it.
  await open(page, a.id);
  await page.getByRole('button', { name: 'Add server' }).first().click();
  const form = page.getByRole('dialog').filter({ hasText: 'Add MCP server' });
  await form.getByLabel('Name', { exact: true }).fill(name);
  await form.getByPlaceholder('npx').fill(process.execPath);
  await form.locator('textarea').first().fill(PROBE);
  await form.getByLabel('Available in').selectOption('selected');
  await expect(form.getByRole('button', { name: 'Add server', exact: true })).toBeDisabled();
  await form.getByLabel('Available in').selectOption('all');
  await expect(form).toContainText('Also every workspace added later (not Scratch).');
  await form.getByRole('checkbox', { name: /Enable now/ }).check();
  await shot(page, 'add-form-all');
  await form.getByRole('button', { name: 'Add server', exact: true }).click();
  const confirm = page.getByRole('dialog').filter({ hasText: 'Share MCP server' });
  await expect(confirm).toContainText('every workspace except Scratch — also ones added later —');
  await expect(confirm).toContainText('can’t read or change them');
  await shot(page, 'share-confirm');
  await confirm.getByRole('button', { name: 'Share', exact: true }).click();
  const own = row(page, name);
  await expect(own.getByTestId('mcp-server-scope')).toHaveText('All workspaces', { timeout: 30_000 });
  await own.getByRole('button', { name: 'Discover', exact: true }).click();
  await expect(own.locator('[data-label="Tools"]')).not.toHaveText('0', { timeout: 30_000 });

  // C is created AFTER the server: it has it, from A — and so do its sessions.
  const c = await workspace(`Scope C ${info.project.name}`);
  await open(page, c.id);
  await expect(row(page, name).getByTestId('mcp-server-scope')).toHaveText(`Shared from Scope A ${info.project.name}`, { timeout: 30_000 });
  await expectNoHorizontalOverflow(page);
  await shot(page, 'list-shared-from');
  expect(await sessionTools(page, c.id)).toContain(`mcp__${name}__probe_link`);

  // Narrow to "Selected: A, B" (from C — the owner may): gone from C.
  await open(page, c.id);
  await row(page, name).getByRole('button', { name: `More actions for ${name}` }).click();
  await page.getByRole('menuitem', { name: 'Available in…' }).click();
  const scope = page.getByRole('dialog').filter({ hasText: 'is available' });
  await scope.getByLabel('Available in').selectOption('selected');
  await expect(scope.getByTestId('mcp-scope-save')).toBeDisabled();
  await scope.getByRole('checkbox', { name: `Scope B ${info.project.name}` }).check();
  await shot(page, 'scope-dialog-selected');
  await scope.getByTestId('mcp-scope-save').click();
  await expect(row(page, name)).toHaveCount(0, { timeout: 30_000 });
  expect(await sessionTools(page, c.id)).not.toContain(`mcp__${name}__`);
  // …and still in B, shared from A.
  await open(page, b.id);
  await expect(row(page, name).getByTestId('mcp-server-scope')).toHaveText(`Shared from Scope A ${info.project.name}`, { timeout: 30_000 });
});

test('a direct server stays in its home: the control says why, the API refuses', async ({ page }, info) => {
  const a = await workspace(`Native A ${info.project.name}`);
  await open(page, a.id);
  await page.getByRole('button', { name: 'Add server' }).first().click();
  const form = page.getByRole('dialog').filter({ hasText: 'Add MCP server' });
  await form.getByTestId('mcp-server-runs').selectOption('native');
  const control = form.getByLabel('Available in');
  await expect(control).toBeDisabled();
  await expect(control).toHaveValue('this');
  await expect(form).toContainText('A direct server stays in its home workspace. Run it through Otto to share it.');
  await control.scrollIntoViewIfNeeded();
  await shot(page, 'native-add-form');
  await form.getByRole('button', { name: 'Cancel', exact: true }).click();

  const made = await ctx.post(`${base}/api/v1/workspaces/${a.id}/mcp-servers`, { data: { name: `native-${info.project.name}`, command: 'true', enabled: true } });
  expect(made.ok(), await made.text()).toBeTruthy();
  const id = (await made.json()).id as string;
  const share = await ctx.patch(`${base}/api/v1/mcp/servers/${id}`, { data: { all_workspaces: true } });
  expect(share.status()).toBe(400);
  expect(await share.text()).toContain('run it through Otto');
});
