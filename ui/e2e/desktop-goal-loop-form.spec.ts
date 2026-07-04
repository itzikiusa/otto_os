import { test, expect } from '@playwright/test';
import { apiCtx, seedWorkspace } from './seed';
import { openPage, expectNoHorizontalOverflow } from './helpers';

// New-goal-loop form E2E (focused): the repository FolderPicker and the form's
// always-visible controls. "Define with AI" is NOT driven here — it needs a live
// CLI; the executor provider/model override logic it gates is unit-tested and
// exercised end-to-end via the run-with-otto launch API.

let wsA = '';

test.beforeAll(async () => {
  const a = await apiCtx();
  wsA = await seedWorkspace(a.ctx, a.base);
  await a.ctx.dispose();
});

test.beforeEach(async ({ page }, testInfo) => {
  test.skip(testInfo.project.name !== 'desktop-browser', 'desktop-browser only');
  await page.addInitScript((wsId) => {
    localStorage.setItem('otto_workspace', wsId as string);
    localStorage.setItem('otto_rail_expanded', '0');
  }, wsA);
});

test.describe('goal-loop form', () => {
  test('Browse… opens the daemon folder picker for the repository path', async ({ page }) => {
    await openPage(page, 'loops');
    await page
      .getByRole('button', { name: /New goal loop|Define your first goal/ })
      .first()
      .click();

    // The form renders with the repo path input + Browse….
    const repoInput = page.getByPlaceholder('/absolute/path/to/repo');
    await expect(repoInput).toBeVisible();
    await page.getByRole('button', { name: 'Browse…' }).click();

    // The shared FolderPicker modal opens, browsing the daemon host (~).
    const picker = page.getByLabel('Choose a repository');
    await expect(picker).toBeVisible();
    await expect(picker.locator('.crumb')).toBeVisible();
    await picker.getByRole('button', { name: 'Cancel' }).click();
    await expect(picker).toHaveCount(0);

    // A typed path + goal enables Define with AI (the AI step itself is not driven).
    await repoInput.fill('/tmp/some-repo');
    await page
      .getByPlaceholder('e.g. Make the export endpoint stream instead of buffering, and add a test.')
      .fill('E2E: form gate check');
    await expect(page.getByRole('button', { name: 'Define with AI' })).toBeEnabled();
    await expectNoHorizontalOverflow(page);
  });
});
