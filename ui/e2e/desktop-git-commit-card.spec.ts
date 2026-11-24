import { test, expect } from '@playwright/test';
import { expectFullyInViewport, openPage } from './helpers';
import { expectChangesFirst, remoteSubject, seedCardRepo, stubForge, type CardRepo } from './git-commit-card-fixture';

// Changes first (G1): a dirty repository opens on Changes with the commit
// card pinned on top — every file included, the message drafted — and ONE
// primary that moves Commit and push → Create pull request → Open pull request.
// Open → read → one click.

const NAME = 'e2e-commit-card';
// The forge is stubbed with page.route: keep the app's service worker out
// of the way (on WebKit its fetches bypass the routes).
test.use({ serviceWorkers: 'block' });

let repo: CardRepo;

test.beforeAll(async () => {
  repo = await seedCardRepo(NAME);
});

test('dirty repo: Changes first → Commit and push → Create pull request', async ({ page }, info) => {
  test.skip(info.project.name !== 'desktop-browser', 'desktop-browser project only');
  await page.setViewportSize({ width: 1440, height: 900 });
  let open: unknown[] = [];
  await stubForge(page, () => open);
  await openPage(page, 'git');
  await page.locator('.git-tab-new').click();
  const menu = page.locator('.ctx-menu');
  await menu.locator('.ctx-search-input').fill(NAME);
  await menu.getByRole('menuitem', { name: NAME }).first().click();

  await expectChangesFirst(page);
  const card = page.getByTestId('commit-card');
  await expectFullyInViewport(page, card, 'commit card');
  // The first file's diff is open beside the list.
  await expect(page.locator('.wip-panel .wp-diff')).toBeVisible();
  // One primary; Commit only is the secondary.
  await expect(card.locator('.btn.primary')).toHaveCount(1);
  await expect(card.locator('.wc-commit')).toHaveText(/Commit and push/);
  await expect(card.locator('.wc-commit-only')).toHaveText('Commit only');

  // One click: committed AND pushed to the remote.
  await card.locator('.wc-commit').click();
  await expect.poll(() => remoteSubject(repo), { timeout: 20_000 }).toBe('feat: ship the commit card');

  // The card now offers the next step: the pull request.
  const next = page.getByTestId('commit-card-primary');
  await expect(next).toHaveAttribute('data-kind', 'create-pr', { timeout: 15_000 });
  await expect(next).toHaveText(/Create pull request/);
  await next.click();
  const sheet = page.getByRole('dialog', { name: 'New pull request' });
  await expect(sheet).toBeVisible();
  await sheet.getByRole('button', { name: 'Cancel' }).click();

  // With a PR open for the branch the primary opens it (Pull requests view).
  open = [{ number: 42, title: 'Ship the commit card', state: 'open', source_branch: repo.branch, target_branch: 'main', url: 'https://github.com/acme/x/pull/42' }];
  await page.reload();
  await expect(page.locator('.rv-tab[aria-selected="true"]')).toContainText('History', { timeout: 15_000 });
  await page.locator('.rv-tab', { hasText: 'Changes' }).click();
  await expect(next).toHaveAttribute('data-kind', 'pr', { timeout: 15_000 });
  await expect(next).toHaveText(/Open pull request/);
  await next.click();
  await expect(page.locator('.rv-tab[aria-selected="true"]')).toContainText('Pull requests');
});
