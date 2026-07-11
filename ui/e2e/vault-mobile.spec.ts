import { expect, test } from '@playwright/test';
import { apiCtx, seedWorkspace, seedVaultDir } from './seed';
import { expectNoHorizontalOverflow, openPage } from './helpers';

// Vault v3 (docs home) — PHONE/TABLET usability. On ≤800px the three-pane
// desktop layout stacks: the left panel becomes a top strip (tree capped at
// 40% height), the right panel hides, and the note view takes the rest. These
// specs assert the page fits every mobile viewport, the tree is reachable, and
// opening a note renders a scrollable reading view — against a REAL on-disk
// seeded vault.

let workspaceId = '';
let vaultId = 0;

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

test('vault page fits the viewport with a seeded vault', async ({ page }) => {
  await openPage(page, 'vault');
  await expect(page.locator('.vault-page')).toBeVisible({ timeout: 15_000 });
  await expectNoHorizontalOverflow(page);
});

test('tree is reachable and opening a note renders it', async ({ page }) => {
  await openPage(page, 'vault');
  const tree = page.locator('.tree');
  await expect(tree.getByText('services', { exact: true })).toBeVisible({ timeout: 15_000 });
  await tree.getByText('services', { exact: true }).click();
  await tree.getByText('auth-api', { exact: true }).click();
  await expect(page.locator('.read').getByRole('heading', { name: 'Overview' })).toBeVisible();
  await expectNoHorizontalOverflow(page);
});
