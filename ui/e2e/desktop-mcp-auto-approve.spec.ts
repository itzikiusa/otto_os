import { test, expect, type APIRequestContext } from '@playwright/test';
import { apiCtx, seedGitRepo, seedWorkspace } from './seed';
import { enableMcpFixtureCatalog } from './mcp-catalog-fixture';

// MCP → Otto server → Auto-approve: the opt-in rules that let a mutating
// otto.* tool (create_pr) run without a per-call approval. Pins the catalog
// switch (confirm → a global per-tool rule shown in the panel), the panel's
// delete, and the irreversible guardrail in the New-rule form (merge_pr can't
// be saved until the second toggle is ticked), and the approval card's
// "Approve & always allow…" (prefilled rule form → rule + approval).

let base = '';
let workspaceId = '';

test.describe.configure({ mode: 'serial' });

async function rules(ctx: APIRequestContext): Promise<{ id: string; target: string }[]> {
  const r = await ctx.get(`${base}/api/v1/mcp/auto-approve`);
  expect(r.ok(), `GET auto-approve → ${r.status()} ${await r.text()}`).toBeTruthy();
  return ((await r.json()) as { rules: { id: string; target: string }[] }).rules;
}

async function clearRules(ctx: APIRequestContext): Promise<void> {
  for (const rule of await rules(ctx)) await ctx.delete(`${base}/api/v1/mcp/auto-approve/${rule.id}`);
}

test.beforeAll(async () => {
  const seeded = await apiCtx();
  base = seeded.base;
  workspaceId = await seedWorkspace(seeded.ctx, base);
  await enableMcpFixtureCatalog(seeded.ctx, base);
  await clearRules(seeded.ctx);
  await seeded.ctx.dispose();
});

test.afterAll(async () => {
  const { ctx } = await apiCtx();
  await clearRules(ctx);
  await ctx.dispose();
});

test.beforeEach(async ({ page }) => {
  await page.addInitScript((wsId) => {
    if (!sessionStorage.getItem('mcp-aa-seeded')) {
      localStorage.setItem('otto_workspace', wsId as string);
      sessionStorage.setItem('mcp-aa-seeded', '1');
    }
  }, workspaceId);
});

test('catalog switch auto-approves create_pr and the panel lists + deletes it', async ({ page }) => {
  await page.goto('/#/mcp');
  const panel = page.locator('[data-testid="mcp-auto-approve-panel"]');
  await expect(panel).toBeVisible({ timeout: 30_000 });
  await expect(panel.getByText('No auto-approve rules')).toBeVisible();

  const toggle = page.locator('input[data-testid="mcp-auto-otto.create_pr"]');
  await expect(toggle).not.toBeChecked();
  await toggle.click();
  await page.getByRole('dialog').getByRole('button', { name: 'Auto-approve', exact: true }).click();
  await expect(toggle).toBeChecked();
  const row = panel.locator('[data-testid="mcp-auto-approve-rule-create_pr"]');
  await expect(row).toBeVisible();
  await expect(row).toContainText('Everywhere');

  await row.getByRole('button', { name: /^Delete / }).click();
  await page.getByRole('dialog').getByRole('button', { name: 'Delete', exact: true }).click();
  await expect(row).toHaveCount(0);
  await expect(toggle).not.toBeChecked();
});

test('an irreversible tool needs the second toggle in the New-rule form', async ({ page }) => {
  await page.goto('/#/mcp');
  await page.locator('[data-testid="mcp-auto-approve-new"]').click();
  const form = page.locator('[data-testid="mcp-auto-approve-form"]');
  await expect(form).toBeVisible();
  await form.getByRole('radio', { name: 'One tool' }).check();
  await form.locator('[data-testid="mcp-auto-approve-tool"]').selectOption('merge_pr');
  const save = page.locator('[data-testid="mcp-auto-approve-save"]');
  await expect(save).toBeDisabled();
  await form.locator('[data-testid="mcp-auto-approve-ack"]').check();
  await expect(save).toBeEnabled();
  await save.click();
  await expect(
    page.locator('[data-testid="mcp-auto-approve-rule-merge_pr"]').getByText('Irreversible allowed'),
  ).toBeVisible();
});

test('"Approve & always allow…" on a pending card creates the rule and approves it', async ({ page }) => {
  const { ctx } = await apiCtx();
  try {
    const { repoId } = await seedGitRepo(ctx, base, workspaceId);
    const args = { repo_id: repoId, title: 'E2E always allow', description: 'Body',
                   source_branch: 'e2e/always', target_branch: 'main' };
    const call = () => ctx.post(`${base}/api/v1/mcp/otto-tools/invoke`, {
      data: { tool: 'otto.create_pr', arguments: args },
    });
    const first = (await (await call()).json()) as { decision: string; approval_id?: string };
    expect(first.decision).toBe('pending_approval');
    // A retry reuses the waiting card instead of filing another.
    const again = (await (await call()).json()) as { approval_id?: string };
    expect(again.approval_id).toBe(first.approval_id);
  } finally {
    await ctx.dispose();
  }

  await page.goto('/#/mcp/activity');
  const queue = page.locator('[data-testid="mcp-approvals"]');
  await expect(queue).toBeVisible({ timeout: 30_000 });
  await expect(queue.getByText('otto MCP server → otto.create_pr')).toHaveCount(1);
  await queue.locator('[data-testid="mcp-approve-always"]').first().click();
  const form = page.locator('[data-testid="mcp-auto-approve-form"]');
  await expect(form).toBeVisible();
  await expect(form.getByRole('radio', { name: 'One tool' })).toBeChecked();
  await expect(form.locator('[data-testid="mcp-auto-approve-tool"]')).toHaveValue('create_pr');
  await page.locator('[data-testid="mcp-auto-approve-save"]').click();
  await expect(form).toHaveCount(0);
  await expect(queue.getByText('otto MCP server → otto.create_pr')).toHaveCount(0);
  const check = await apiCtx();
  expect((await rules(check.ctx)).some((r) => r.target === 'create_pr')).toBeTruthy();
  await check.ctx.dispose();
});
