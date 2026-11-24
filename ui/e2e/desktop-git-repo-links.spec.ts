import { test, expect } from '@playwright/test';
import { seedCardRepo } from './git-commit-card-fixture';

// Two real-data Git findings: a GitHub repo with no linked git account says
// THAT (not "the provider is unreachable"), and a deep link to a repo id the
// repo list doesn't have shows "Repository not found" instead of leaving the
// previous repo on screen under the other repo's URL.

test.use({ serviceWorkers: 'block' });

test.beforeEach(async ({ page }, info) => {
  test.skip(info.project.name !== 'desktop-browser', 'desktop-browser only');
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.addInitScript(() => localStorage.setItem('otto_firstrun_dismissed', '1'));
});

test('Pull requests on a GitHub repo with no git account: says what is wrong and how to fix it', async ({ page }) => {
  const repo = await seedCardRepo('e2e-no-account');
  await page.goto('/#/git/' + repo.id + '/prs');
  const err = page.getByText(/No git account is linked to this repo/);
  await expect(err).toBeVisible({ timeout: 30_000 });
  await expect(err).toContainText('Settings → Git Accounts');
  await expect(page.getByText(/unreachable/i)).toHaveCount(0);
});

test('a deep link to an unknown repo id: "Repository not found", not the previous repo', async ({ page }) => {
  const repo = await seedCardRepo('e2e-deeplink');
  await page.goto('/#/git/' + repo.id + '/graph');
  await expect(page.locator('.refs-panel')).toBeVisible({ timeout: 30_000 });
  await page.goto('/#/git/00000000-0000-0000-0000-00000000dead/graph');
  await expect(page.getByRole('heading', { name: 'Repository not found' })).toBeVisible({ timeout: 15_000 });
  await expect(page.locator('.refs-panel')).toHaveCount(0);
  await expect(page.locator('.git-header .ph-actions').getByRole('button', { name: /^Pull\b/ })).toHaveCount(0);
  await page.getByRole('button', { name: 'Show repositories' }).click();
  await expect(page).toHaveURL(/#\/git$/);
});
