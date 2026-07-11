import { expect, test } from '@playwright/test';
import { apiCtx, seedWorkspace, seedVaultDir } from './seed';
import { openPage } from './helpers';

// Vault docs agents — desktop, against the ISOLATED daemon (OTTO_E2E=1 stubs
// the agent sessions, so runs complete instantly with no real CLI spawns).
// API half: run lifecycle (multi-agent → summarizer → done), refine turn +
// session registry. UI half: the ✨ panel launches a run, per-agent rows show
// provider chips + status pills and reach a terminal state; the refine drawer
// mounts on an open note.

let workspaceId = '';
let vaultId = 0;

test.describe.configure({ mode: 'serial' });

test.beforeAll(async () => {
  const { ctx, base } = await apiCtx();
  workspaceId = await seedWorkspace(ctx, base);
  ({ vaultId } = await seedVaultDir(ctx, base, workspaceId));
  await ctx.dispose();
});

test.beforeEach(async ({ page }) => {
  await page.addInitScript(
    ({ wsId, vId }) => {
      localStorage.setItem('otto_workspace', wsId);
      localStorage.setItem(`otto_vault_last:${wsId}`, String(vId));
      localStorage.setItem('otto_rail_expanded', '0');
    },
    { wsId: workspaceId, vId: vaultId },
  );
});

test('api: multi-agent run reaches done with a summarizer stage', async () => {
  const { ctx, base } = await apiCtx();
  const run = await (
    await ctx.post(
      `${base}/api/v1/workspaces/${workspaceId}/vault/vaults/${vaultId}/docs-agents/run`,
      {
        data: {
          prompt: 'Document the auth flow',
          target_dir: 'services',
          agents: [{ provider: 'claude' }, { provider: 'claude', model: 'sonnet' }],
          summarizer: { provider: 'claude' },
        },
      },
    )
  ).json();
  expect(run.id).toBeTruthy();
  expect(run.agents).toHaveLength(2);
  expect(run.agents[1].model).toBe('sonnet');

  // Stubbed sessions complete instantly — poll to a terminal state.
  let final = run;
  for (let i = 0; i < 60 && ['running', 'summarizing'].includes(final.state); i++) {
    await new Promise((r) => setTimeout(r, 250));
    final = await (await ctx.get(`${base}/api/v1/vault/docs-agents/runs/${run.id}`)).json();
  }
  expect(final.state).toBe('done');
  expect(final.agents.every((a: { state: string }) => a.state === 'done')).toBe(true);
  expect(final.summarizer.state).toBe('done');
  expect(final.agents.every((a: { session_id: string | null }) => !!a.session_id)).toBe(true);
  await ctx.dispose();
});

test('api: single-agent run skips the summarizer; refine registers a session', async () => {
  const { ctx, base } = await apiCtx();
  const v1 = `${base}/api/v1/workspaces/${workspaceId}/vault/vaults/${vaultId}`;
  const run = await (
    await ctx.post(`${v1}/docs-agents/run`, {
      data: { prompt: 'Document deploys', agents: [{ provider: 'claude' }] },
    })
  ).json();
  let final = run;
  for (let i = 0; i < 60 && ['running', 'summarizing'].includes(final.state); i++) {
    await new Promise((r) => setTimeout(r, 250));
    final = await (await ctx.get(`${base}/api/v1/vault/docs-agents/runs/${run.id}`)).json();
  }
  expect(final.state).toBe('done');
  expect(final.summarizer.state).toBe('skipped');

  // Refine: long request returns a reply + session id; registry serves it back.
  const refine = await (
    await ctx.post(`${v1}/docs-agents/refine`, {
      data: { path: 'runbooks/deploy.md', prompt: 'Add a rollback section' },
    })
  ).json();
  expect(refine.session_id).toBeTruthy();
  const reg = await (
    await ctx.get(`${v1}/docs-agents/refine-session?path=runbooks%2Fdeploy.md`)
  ).json();
  expect(reg.session_id).toBe(refine.session_id);
  await ctx.dispose();
});

test('ui: docs-agents panel runs and shows per-agent rows', async ({ page }) => {
  await openPage(page, 'vault');
  await page.locator('button[title^="Docs agent"]').click();

  const panel = page.locator('.docs-agents');
  await expect(panel).toBeVisible({ timeout: 15_000 });
  await panel.locator('textarea').fill('Document the services in this vault');
  // Two agents: the form starts with one row — add another.
  await panel.getByRole('button', { name: /add agent/i }).click();
  await panel.getByRole('button', { name: /^Run$/i }).click();

  // Per-agent rows with provider chips appear and reach a terminal state.
  await expect(panel.locator('.agent-row, .agent-card').first()).toBeVisible({ timeout: 15_000 });
  await expect(panel).toContainText('claude');
  await expect(panel).toContainText(/done|error/i, { timeout: 30_000 });
});

test('ui: refine drawer mounts on an open note', async ({ page }) => {
  await openPage(page, 'vault');
  const tree = page.locator('.tree');
  await tree.getByText('runbooks', { exact: true }).click();
  await tree.getByText('deploy', { exact: true }).click();
  await page.locator('button[title="Refine with AI"]').click();
  const drawer = page.locator('.refine-drawer');
  await expect(drawer).toBeVisible();
  await expect(drawer.locator('select')).toBeVisible();
  await drawer.locator('input[type="text"], input:not([type])').first().fill('Tighten the intro');
  await drawer.getByRole('button', { name: /send/i }).click();
  // Stubbed turn completes; the drawer stays open and the input re-enables.
  await expect(drawer.locator('input').first()).toBeEnabled({ timeout: 20_000 });
});
