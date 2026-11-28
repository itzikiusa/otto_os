import { test, expect, type Page } from '@playwright/test';
import { mkdirSync } from 'node:fs';
import { join } from 'node:path';

// ─────────────────────────────────────────────────────────────────────────────
// rc.6 side surfaces — the Git pull-request page at 1440×900 (MOCK mode): the
// wide list pane (Pull requests · count · + · ⋯), the PR, and the Detail pane
// (Details | Agent); ⋯ › Hide Pull requests, Show in the page header, ⌥⌘B.
// The behaviour is pinned by desktop-git-pr-workspace.spec.ts; this spec adds
// the keyboard walk and the pictures (OTTO_E2E_SHOTS=1 → sidebars/shots/rc6-side-c/).
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

test.beforeEach(async ({ page }, info) => {
  test.skip(info.project.name !== 'desktop-browser', 'desktop-browser only');
  await page.addInitScript(() => {
    localStorage.setItem('otto_mock', '1');
    localStorage.setItem('otto_rail_expanded', '0');
  });
  await page.setViewportSize({ width: 1440, height: 900 });
});

test('list pane, PR and Detail pane; the keyboard walks the rows; ⌥⌘B closes and opens the pane', async ({ page }) => {
  await page.goto('/#/git/rep_otto/pr/51');
  await expect(page.locator('.prd-title')).toBeVisible({ timeout: 15_000 });
  const list = page.getByRole('complementary', { name: 'Pull requests' });
  await expect(list.locator('.sph-count')).toHaveText(/^\d+$/);
  await expect(page.getByTestId('pr-details-card')).toBeVisible();
  await shot(page, 'prs-desktop-detail');

  // ↓ / End walk the rows (the pane's one keyboard walk); Enter opens.
  await list.locator('.tr-main').first().focus();
  await page.keyboard.press('ArrowDown');
  await page.keyboard.press('Enter');
  await expect(page.locator('.prd[data-pr="50"] .prd-title')).toBeVisible({ timeout: 15_000 });
  await expect(page.locator('.pr-row[data-pr="50"] .tr-main')).toHaveAttribute('aria-current', 'true');

  // ⌥⌘B: the Detail pane closes and comes back.
  await page.keyboard.press('Alt+Meta+b');
  await expect(page.getByTestId('pr-details-card')).toHaveCount(0);
  await shot(page, 'prs-desktop-content');
  await page.keyboard.press('Alt+Meta+b');
  await expect(page.getByTestId('pr-details-card')).toBeVisible();

  // The Agent view shares the pane.
  await page.getByTestId('pr-pane-tabs').getByRole('tab', { name: 'Agent' }).click();
  await expect(page.getByTestId('pr-agent-panel')).toBeVisible();
  await shot(page, 'prs-desktop-agent');

  await list.getByTestId('pr-list-more').click();
  await page.locator('.ctx-menu').getByText('Hide Pull requests').click();
  await expect(page.getByRole('button', { name: 'Show Pull requests' })).toBeVisible();
  await shot(page, 'prs-desktop-list-hidden');
  await page.getByRole('button', { name: 'Show Pull requests' }).click();
  await expect(list).toBeVisible();
});
