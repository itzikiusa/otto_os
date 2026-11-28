import { test, expect, type Page } from '@playwright/test';
import { mkdirSync } from 'node:fs';
import { join } from 'node:path';
import { expectNoHorizontalOverflow } from './helpers';

// ─────────────────────────────────────────────────────────────────────────────
// rc.6 side surfaces — the Git pull-request page on a phone and a tablet, with
// real taps (MOCK mode, ui/src/lib/api/mock-prs.ts):
//   phone   the PR list IS the page (☰ · Pull requests n · + · ⋯ · Side panel;
//           the repositories under ⋯); a PR is pushed with
//           ‹ Pull requests; Back keeps it marked; another PR; ⓘ is a
//           full-width sheet; the Side panel stays one tap away.
//   tablet  the list is a column beside the PR (the content keeps 480 px);
//           the Detail pane has no room, so ⓘ opens it as a right sheet.
// OTTO_E2E_SHOTS=1 writes light/dark pictures to sidebars/shots/rc6-side-c/.
// ─────────────────────────────────────────────────────────────────────────────

const SHOTS = '/Users/itziklavon/.claude/projects/-Users-itziklavon-claude-ade/workflows-effort/sidebars/shots/rc6-side-c';
async function shot(page: Page, name: string): Promise<void> {
  if (!process.env.OTTO_E2E_SHOTS) return;
  mkdirSync(SHOTS, { recursive: true });
  for (const scheme of ['light', 'dark'] as const) {
    await page.emulateMedia({ colorScheme: scheme });
    await page.waitForTimeout(150); // ui-guards: allow — let the theme repaint before the picture
    await page.screenshot({ path: join(SHOTS, `${name}-${scheme}.png`), animations: 'disabled' });
  }
  await page.emulateMedia({ colorScheme: 'light' });
}

test.beforeEach(async ({ page }) => {
  await page.addInitScript(() => localStorage.setItem('otto_mock', '1'));
});

const header = (page: Page) => page.locator('.gitpage > [data-testid="page-header"]');
const row = (page: Page, n: number) => page.locator(`.pr-row[data-pr="${n}"] .tr-main`);

test('phone: list → PR → ‹ Pull requests → another PR; details sheet; ⋯ views; Side panel', async ({ page }, info) => {
  test.skip(info.project.name !== 'iphone-portrait', 'iPhone portrait');
  await page.setViewportSize({ width: 390, height: 844 });
  // A deep link lands on the PR (pushed); Back is the list.
  await page.goto('/#/git/rep_otto/pr/51');
  await expect(page.locator('.prd-title')).toBeVisible({ timeout: 15_000 });
  const back = header(page).getByRole('button', { name: 'Back to Pull requests' });
  await expect(back).toBeVisible();
  await expect(header(page).getByRole('button', { name: 'Open navigator' })).toHaveCount(0);
  await expect(page.locator('.mtopbar'), 'one header row').toHaveCount(0);
  await expect(page.locator('.prw-back'), 'no hand-made back').toHaveCount(0);
  // ‹ Pull requests · #51 title: the pull request, not the repository switcher (that is under ⋯).
  await expect(header(page).locator('.ph-title')).toHaveText(/^\s*#51 \S/);
  await expect(header(page).getByTestId('git-phone-repo')).toHaveCount(0);
  await header(page).getByRole('button', { name: 'More actions' }).tap();
  await expect(page.locator('.ctx-menu').getByText('Repository', { exact: true })).toBeVisible();
  await page.keyboard.press('Escape');
  await shot(page, 'prs-phone-content');

  await back.tap();
  await expect(page.locator('.pr-search')).toBeVisible();
  await expect(page.locator('.prd-title')).toBeHidden();
  await expect(header(page).locator('.ph-title')).toHaveText('Pull requests');
  await expect(header(page).getByRole('button', { name: 'Open navigator' })).toBeVisible();
  await expect(header(page).getByRole('button', { name: 'New pull request' }), 'the pane\'s one primary').toBeVisible();
  await expect(row(page, 51), 'the PR you came from stays marked').toHaveAttribute('aria-current', 'true');
  await expectNoHorizontalOverflow(page);
  await shot(page, 'prs-phone-list');

  // ⋯ carries the repositories (the title's switcher) and the page's rows (no Hide on a phone);
  // the History · Pull requests · Review switch keeps its row.
  await expect(page.getByRole('tablist', { name: 'otto views' })).toBeVisible();
  await header(page).getByRole('button', { name: 'More actions' }).tap();
  const menu = page.locator('.ctx-menu');
  await expect(menu.getByText('Repository', { exact: true })).toBeVisible();
  await expect(menu.getByText('Refresh', { exact: true })).toBeVisible();
  await expect(menu.getByText('Hide Pull requests')).toHaveCount(0);
  await page.keyboard.press('Escape');

  await row(page, 50).tap();
  await expect(page.locator('.prd[data-pr="50"] .prd-title')).toBeVisible({ timeout: 15_000 });
  await expect(back).toBeVisible();
  // ⓘ: the details card as a full-width sheet.
  await page.getByTestId('pr-details-toggle').tap();
  const sheet = page.getByRole('dialog', { name: 'Pull request details' });
  await expect(sheet.getByTestId('pr-details-card')).toBeVisible();
  expect((await sheet.boundingBox())!.width, 'full width on a phone').toBeGreaterThanOrEqual(389);
  await shot(page, 'prs-phone-detail');
  await sheet.getByRole('button', { name: 'Close Pull request details' }).tap();
  await expect(sheet).toBeHidden();

  await back.tap();
  await expect(row(page, 50)).toHaveAttribute('aria-current', 'true');
  await header(page).getByRole('button', { name: 'Side panel' }).tap();
  await expect(page.locator('.drawer[aria-label="Side panel"]')).toBeVisible();
});

test('tablet: the list is a column beside the PR; ⓘ opens a right sheet', async ({ page }, info) => {
  test.skip(info.project.name !== 'ipad-portrait', 'iPad portrait');
  await page.setViewportSize({ width: 820, height: 1180 });
  await page.goto('/#/git/rep_otto/pr/51');
  await expect(page.locator('.prd-title')).toBeVisible({ timeout: 15_000 });
  const list = page.getByRole('complementary', { name: 'Pull requests' });
  await expect(list).toBeVisible();
  const lb = (await list.boundingBox())!;
  const cb = (await page.locator('.prw-detail').boundingBox())!;
  expect(lb.x + lb.width).toBeLessThanOrEqual(cb.x + 2);
  expect(cb.width, 'the content keeps 480 px').toBeGreaterThanOrEqual(470);
  await expect(header(page).getByRole('button', { name: 'Side panel', exact: true })).toBeVisible();
  await expect(page.getByTestId('pr-details-card'), 'no room: the pane waits for ⓘ').toHaveCount(0);
  await expectNoHorizontalOverflow(page);
  await shot(page, 'prs-tablet-list');
  await row(page, 50).tap();
  await expect(page.locator('.prd[data-pr="50"] .prd-title')).toBeVisible({ timeout: 15_000 });
  await expect(row(page, 50)).toHaveAttribute('aria-current', 'true');
  await page.getByTestId('pr-details-toggle').tap();
  const sheet = page.getByRole('dialog', { name: 'Pull request details' });
  await expect(sheet.getByTestId('pr-details-card')).toBeVisible();
  await shot(page, 'prs-tablet-detail');
  await sheet.getByRole('button', { name: 'Close Pull request details' }).tap();
  await expect(sheet).toBeHidden();
});
