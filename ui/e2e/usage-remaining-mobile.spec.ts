// Usage ▸ Remaining on a phone (390 px portrait / landscape): Usage is one of
// the five tabs of the phone light version, so the per-login limits must read
// at that width — every window's name, bar and "N% left · resets in …" line
// fits without horizontal scroll, the stale verdict stays visible, and the
// light version reaches the section too.
import { test, expect, type Page } from '@playwright/test';
import { apiCtx } from './seed';
import { openPage, expectNoHorizontalOverflow } from './helpers';
import { cannedLimits, claudeLimits, remaining, routeLimits, seedCodexLimits } from './usage-remaining-fixture';

const phone = (page: Page) => page.evaluate(() => window.innerWidth <= 640 || window.innerHeight <= 500);

/** Every element of `sel` inside the section lies within the viewport width. */
async function expectInsideWidth(page: Page, sel: string): Promise<void> {
  const out = await remaining(page)
    .locator(sel)
    .evaluateAll((els) => els.filter((e) => e.getBoundingClientRect().right > window.innerWidth + 1).length);
  expect(out, `${sel} past the viewport's right edge`).toBe(0);
}

test('the loaded Codex row fits a phone and keeps every window readable', async ({ page }) => {
  const { ctx, base } = await apiCtx();
  await seedCodexLimits(ctx, base);
  await openPage(page, 'usage');
  test.skip(!(await phone(page)), 'phone layouts only');
  const row = remaining(page).getByTestId('limits-row-codex:default');
  await expect(row).toBeVisible();
  await row.scrollIntoViewIfNeeded();
  for (const id of ['primary', 'secondary']) {
    const win = row.getByTestId(`limits-window-${id}`);
    await expect(win).toContainText(/% left · resets in/);
    const bar = win.getByRole('meter');
    const box = await bar.boundingBox();
    expect(box, `${id} bar`).not.toBeNull();
    expect(box!.width, `${id} bar keeps a usable width`).toBeGreaterThan(40);
  }
  await expectInsideWidth(page, '.win, .win-text, .acct-head');
  await expectNoHorizontalOverflow(page);
});

test('stale and unknown rows stay legible on a phone', async ({ page }) => {
  await routeLimits(page, cannedLimits());
  await openPage(page, 'usage');
  test.skip(!(await phone(page)), 'phone layouts only');
  const section = remaining(page);
  const stale = section.getByTestId('limits-row-codex:account:acc-work');
  await expect(stale.getByTestId('limits-stale')).toBeVisible();
  await expect(stale.getByTestId('limits-window-secondary')).toContainText('8% left 5 h ago');
  const unknown = section.getByTestId('limits-row-claude:account:acc-max');
  await expect(unknown.getByTestId('limits-unknown')).toBeVisible();
  await expect(unknown).toContainText('does not write its plan limits');
  await expectInsideWidth(page, '.acct, .reason');
  await expectNoHorizontalOverflow(page);
});

test('the phone light version shows Remaining on its Usage tab', async ({ page }) => {
  await routeLimits(page, cannedLimits());
  await page.addInitScript(() => localStorage.setItem('otto_phone_light', '1'));
  await page.goto('/#/usage');
  await expect(page.locator('.shell')).toBeVisible({ timeout: 30_000 });
  test.skip(!(await phone(page)), 'phone layouts only');
  await expect(remaining(page).getByRole('heading', { name: 'Remaining' })).toBeVisible();
  await expect(remaining(page).getByTestId('limits-row-codex:account:acc-work')).toBeVisible();
  await expectNoHorizontalOverflow(page);
});

test('a checked Claude login, its two weekly windows and its switch fit a phone', async ({ page }) => {
  await routeLimits(page, claudeLimits('ok'));
  await openPage(page, 'usage');
  test.skip(!(await phone(page)), 'phone layouts only');
  const row = remaining(page).getByTestId('limits-row-claude:account:acc-max');
  await row.scrollIntoViewIfNeeded();
  for (const id of ['seven_day', 'seven_day_fable']) {
    const win = row.getByTestId(`limits-window-${id}`);
    await expect(win).toContainText('% left · resets in');
    const box = await win.getByRole('meter').boundingBox();
    expect(box, `${id} bar`).not.toBeNull();
    expect(box!.width, `${id} bar keeps a usable width`).toBeGreaterThan(40);
  }
  await expect(row.getByTestId('limits-window-seven_day_fable')).toContainText('not in effect now');
  const sw = row.getByRole('switch', { name: 'Check Max with Anthropic' });
  await expect(sw).toHaveAttribute('aria-checked', 'true');
  await expect(row.getByRole('button', { name: 'Check now' })).toBeVisible();
  await expectInsideWidth(page, '.acct, .check, .win');
  await expectNoHorizontalOverflow(page);
});
