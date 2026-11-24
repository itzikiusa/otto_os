import { test, expect } from '@playwright/test';
import { expectNoHorizontalOverflow } from './helpers';

// The Git landing and Focus on a phone (G4): repositories are rows with
// words, and a Focus PR title keeps its own line (it used to be cut to
// "#42 Di…" by the chips beside it).

const PHONES = ['iphone-portrait', 'android-phone'];

test.beforeEach(async ({ page }, info) => {
  test.skip(!PHONES.includes(info.project.name), 'phone projects only');
  await page.addInitScript(() => localStorage.setItem('otto_mock', '1'));
});

test('Focus: a pull request title owns its line', async ({ page }) => {
  await page.goto('/#/git/focus');
  const title = page.locator('.fx-pr-text').first();
  await expect(title).toBeVisible({ timeout: 20_000 });
  const vw = page.viewportSize()!.width;
  const box = (await title.boundingBox())!;
  expect(box.width, 'title width').toBeGreaterThan(vw * 0.45);
  await expectNoHorizontalOverflow(page);
});

test('landing: repositories are rows that say where they stand', async ({ page }) => {
  await page.addInitScript(() => localStorage.removeItem('otto_git_open_tabs'));
  await page.goto('/#/git');
  const rows = page.getByTestId('git-repo-row');
  await expect(rows.first()).toBeVisible({ timeout: 20_000 });
  const box = (await rows.first().boundingBox())!;
  expect(box.height).toBeGreaterThanOrEqual(44);
  await expectNoHorizontalOverflow(page);
});
