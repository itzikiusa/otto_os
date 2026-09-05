import { test, expect } from '@playwright/test';
import { apiCtx, seedWorkspace, seedShellSession } from './seed';
import { openPage } from './helpers';

// New Session sheet keyboard flow + terminal auto-focus.
//
// Regression: opening the sheet used to leave keyboard focus wherever it was
// (usually the xterm textarea), so the "(← → to switch)" arrows and any launch
// shortcut went to the terminal instead of the sheet. And a freshly opened
// session never took keyboard focus — typing required a click into the pane.

let wsA = '';

test.beforeAll(async () => {
  const a = await apiCtx();
  wsA = await seedWorkspace(a.ctx, a.base);
  // An existing session so the Agents page renders panes (not the first-run coach).
  await seedShellSession(a.ctx, a.base, wsA);
  await a.ctx.dispose();
});

test.beforeEach(async ({ page }, testInfo) => {
  test.skip(testInfo.project.name !== 'desktop-browser', 'desktop-browser only');
  await page.addInitScript((wsId) => {
    localStorage.setItem('otto_workspace', wsId as string);
    localStorage.setItem('otto_firstrun_dismissed', '1');
  }, wsA);
});

test.describe('new-session keyboard flow', () => {
  test('arrows switch provider, ⌘Enter launches, terminal is focused', async ({ page }) => {
    await openPage(page, 'agents');

    // Open the sheet via ⌘T; fall back to the TabBar + button if the shortcut
    // doesn't reach the app in this browser build.
    const dialog = page.locator('.sheet[role="dialog"][aria-label="New Session"]');
    await page.keyboard.press('Meta+t');
    if (!(await dialog.isVisible().catch(() => false))) {
      await page.getByTitle('New session (⌘T)').click();
    }
    await expect(dialog).toBeVisible();

    // Focus is pulled into the selected provider card on open, so the
    // advertised arrow keys work immediately.
    // (The card is a container: its body button carries the roving tabindex,
    // with the batch-count stepper beside it.)
    const selected = dialog.locator('.provider-card.selected');
    await expect(selected.locator('.card-main')).toBeFocused();
    const before = await selected.locator('.provider-name').innerText();

    // Arrow keys move the selection (and the roving focus).
    await page.keyboard.press('ArrowRight');
    await expect(dialog.locator('.provider-card.selected .card-main')).toBeFocused();
    const after = await dialog.locator('.provider-card.selected .provider-name').innerText();
    expect(after).not.toBe(before);
    await page.keyboard.press('ArrowLeft');
    await expect(dialog.locator('.provider-card.selected .provider-name')).toHaveText(before);

    // Arrows must also work from OUTSIDE the grid (e.g. after clicking a label).
    await dialog.locator('.provider-label').click();
    await page.keyboard.press('ArrowRight');
    await expect(dialog.locator('.provider-card.selected .provider-name')).not.toHaveText(before);

    // Pick the plain shell (no external CLI needed in the throwaway daemon)
    // and launch with ⌘Enter from a text field — the sheet-level shortcut.
    await dialog.locator('.provider-card', { hasText: 'shell' }).locator('.card-main').click();
    await dialog.locator('#ns-title').fill('e2e keys');
    await page.keyboard.press('Meta+Enter');

    // The sheet closes, the new session opens, and the terminal owns the
    // keyboard without any click. (Single tabbed pane → one xterm textarea;
    // note the pane does NOT get the visual `.focused` ring when it's alone.)
    await expect(dialog).toHaveCount(0, { timeout: 10_000 });
    await expect(page.locator('.pane .xterm-helper-textarea')).toBeFocused({ timeout: 10_000 });
  });
});
