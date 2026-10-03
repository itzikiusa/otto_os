import { expect, test } from '@playwright/test';
import { apiCtx, seedWorkspace, seedVaultDir } from './seed';
import { expectFullyInViewport, expectNoHorizontalOverflow, openPage } from './helpers';

// Structured view for typed OKF notes: a Service note gets the metadata
// header, expected-sections checklist, link/backlink chips with hover
// previews and the Live context panel; a plain note keeps the bare markdown;
// the header toggle hides the panels and survives a reload.

let workspaceId = '';
let vaultId = 0;

test.describe.configure({ mode: 'serial' });

test.beforeAll(async () => {
  const { ctx, base } = await apiCtx();
  workspaceId = await seedWorkspace(ctx, base);
  vaultId = (await seedVaultDir(ctx, base, workspaceId)).vaultId;
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

async function openNote(page: import('@playwright/test').Page, folder: string, name: string): Promise<void> {
  const tree = page.locator('.tree');
  await expect(tree.getByText(folder, { exact: true })).toBeVisible({ timeout: 15_000 });
  const leaf = tree.getByText(name, { exact: true });
  if (!(await leaf.isVisible())) await tree.getByText(folder, { exact: true }).click();
  await leaf.click();
}

test('typed note renders structured panels with previews and live context', async ({ page }) => {
  await openPage(page, 'vault');
  await openNote(page, 'services', 'auth-api');

  const panel = page.getByTestId('vault-structured');
  await expect(panel).toBeVisible();
  await expect(panel.locator('.kind')).toHaveText('Service');
  await expect(panel.getByRole('button', { name: '#security' })).toBeVisible();
  await expect(panel.getByText(/expected sections/)).toBeVisible();

  // Outgoing chips: the resolved runbook link; the unresolved one is disabled.
  const deployChip = panel.getByRole('button', { name: 'the deploy runbook' });
  await expect(deployChip).toBeVisible();
  await expect(panel.getByRole('button', { name: 'Missing Note' })).toBeDisabled();
  // Backlinks: Orders API and the deploy runbook link here.
  await expect(panel.getByRole('button', { name: 'Orders API' })).toBeVisible();

  // Hover preview: loads the target's description, stays inside the viewport.
  await deployChip.hover();
  const tip = page.getByRole('tooltip');
  await expect(tip).toContainText('How we ship.');
  await expectFullyInViewport(page, tip);
  await page.mouse.move(0, 0);
  await expect(tip).toHaveCount(0);

  // Inline wikilinks in the body get the same preview.
  await page.locator('.read a.internal-link', { hasText: 'the deploy runbook' }).hover();
  await expect(page.getByTestId('vault-link-preview')).toContainText('How we ship.');
  await expectFullyInViewport(page, tip);

  // Live context resolves to a designed state (no matches in an empty daemon).
  await expect(panel.getByLabel('Live context')).toContainText(/Nothing in Otto matches|Open|Couldn’t read/);
  await expectNoHorizontalOverflow(page);

  // Chip navigates.
  await deployChip.click();
  await expect(page.getByTestId('vault-structured').locator('.kind')).toHaveText('Runbook');
});

test('plain notes stay plain; the toggle hides panels and persists', async ({ page }) => {
  await openPage(page, 'vault');
  await openNote(page, 'runbooks', 'data-model'); // type: Reference → not structured
  await expect(page.locator('.read').getByRole('heading', { name: 'Stores' })).toBeVisible();
  await expect(page.getByTestId('vault-structured')).toHaveCount(0);

  await openNote(page, 'services', 'orders-api');
  await expect(page.getByTestId('vault-structured')).toBeVisible();
  await page.getByRole('button', { name: 'Structured view' }).click();
  await expect(page.getByTestId('vault-structured')).toHaveCount(0);
  await page.reload();
  await openNote(page, 'services', 'orders-api');
  await expect(page.locator('.read')).toContainText('charging the customer card');
  await expect(page.getByTestId('vault-structured')).toHaveCount(0);
  await page.getByRole('button', { name: 'Structured view' }).click();
  await expect(page.getByTestId('vault-structured')).toBeVisible();
});
