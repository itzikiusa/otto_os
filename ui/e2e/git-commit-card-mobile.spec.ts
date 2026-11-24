import { test, expect } from '@playwright/test';
import { expectNoHorizontalOverflow, openPage } from './helpers';
import { expectChangesFirst, remoteSubject, seedCardRepo, stubForge, type CardRepo } from './git-commit-card-fixture';

// The commit card on a phone (G1/G5): a dirty repository opens on Changes,
// the card sits on top with 44 px targets, one tap commits and pushes.

// The forge is stubbed with page.route: keep the app's service worker out
// of the way (on WebKit its fetches bypass the routes).
test.use({ serviceWorkers: 'block' });

let repo: CardRepo;

const PHONES = ['iphone-portrait', 'android-phone'];

test.beforeAll(async ({}, info) => {
  if (PHONES.includes(info.project.name)) repo = await seedCardRepo(`e2e-card-${info.project.name}`);
});

test('phone: Changes first → one tap commits and pushes', async ({ page }, info) => {
  test.skip(!PHONES.includes(info.project.name), 'phone projects only');
  await stubForge(page);
  await openPage(page, `git/${repo.id}`);
  await expectChangesFirst(page);
  const primary = page.getByTestId('commit-card').locator('.wc-commit');
  const box = (await primary.boundingBox())!;
  expect(box.height).toBeGreaterThanOrEqual(44);
  await expectNoHorizontalOverflow(page);
  await primary.tap();
  await expect.poll(() => remoteSubject(repo), { timeout: 20_000 }).toBe('feat: ship the commit card');
  await expect(page.getByTestId('commit-card-primary')).toHaveAttribute('data-kind', 'create-pr', { timeout: 15_000 });
});
