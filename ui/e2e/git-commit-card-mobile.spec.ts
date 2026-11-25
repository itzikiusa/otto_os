import { test, expect } from '@playwright/test';
import { expectNoHorizontalOverflow, openPage } from './helpers';
import { expectWipFirst, remoteSubject, seedCardRepo, stubForge, type CardRepo } from './git-commit-card-fixture';

// The commit card on a phone (G1/G5): a dirty repository opens on History's
// WIP row, the card sits on top with 44 px targets, one tap commits and pushes.

// The forge is stubbed with page.route: keep the app's service worker out
// of the way (on WebKit its fetches bypass the routes).
test.use({ serviceWorkers: 'block' });

let repo: CardRepo;

const PHONES = ['iphone-portrait', 'android-phone'];

test.beforeAll(async ({}, info) => {
  if (PHONES.includes(info.project.name)) repo = await seedCardRepo(`e2e-card-${info.project.name}`);
});

// Leaving one file out of "everything included" keeps the rest included (the
// real-device pass: one uncheck left nothing included and staged nothing).
test('phone: leaving one file out keeps the others included', async ({ page }, info) => {
  test.skip(!PHONES.includes(info.project.name), 'phone projects only');
  const r = await seedCardRepo(`e2e-card-out-${info.project.name}`);
  await stubForge(page);
  await openPage(page, `git/${r.id}`);
  await expectWipFirst(page);
  await page.getByRole('checkbox', { name: /app\.txt$/ }).tap();
  await expect(page.getByTestId('commit-card-included')).toHaveText('1 of 2 included');
  await expect(page.getByRole('checkbox', { name: /notes\.txt$/ })).toBeChecked();
});

test('phone: WIP first → one tap commits and pushes', async ({ page }, info) => {
  test.skip(!PHONES.includes(info.project.name), 'phone projects only');
  await stubForge(page);
  await openPage(page, `git/${repo.id}`);
  await expectWipFirst(page);
  const primary = page.getByTestId('commit-card').locator('.wc-commit');
  const box = (await primary.boundingBox())!;
  expect(box.height).toBeGreaterThanOrEqual(44);
  await expectNoHorizontalOverflow(page);
  // …and nothing inside scrolls sideways either (the WIP list did, by the
  // 2 px a touch hit area overhung its section head: a scrollbar over the nav).
  const sideways = await page.locator('.gitpage-body').evaluate((root) =>
    [root, ...root.querySelectorAll<HTMLElement>('*')]
      .filter((el) => /(auto|scroll)/.test(getComputedStyle(el).overflowX) && el.scrollWidth > el.clientWidth + 1)
      .map((el) => `${el.className} ${el.scrollWidth}>${el.clientWidth}`),
  );
  expect(sideways, 'inner horizontal scrollers').toEqual([]);
  // Finger-sized (44 px): the view switch, Fetch, ⋯, a file's include box and the
  // Included header's Unstage all / Discard all (design review 2, H8).
  for (const t of [page.locator('.git-views .rv-tab').first(), page.locator('.gitpage').getByRole('button', { name: /Fetch/ }), page.locator('.gitpage').getByRole('button', { name: 'More actions', exact: true }), page.locator('.wp-check-hit').first(), ...(await page.locator('.wp-sec-btn').all())]) {
    const b = (await t.boundingBox())!;
    expect(Math.min(b.width, b.height), await t.evaluate((el) => el.className)).toBeGreaterThanOrEqual(44);
  }
  await primary.tap();
  await expect.poll(() => remoteSubject(repo), { timeout: 20_000 }).toBe('feat: ship the commit card');
  await expect(page.getByTestId('commit-card-primary')).toHaveAttribute('data-kind', 'create-pr', { timeout: 15_000 });
});
