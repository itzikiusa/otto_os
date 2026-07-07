import { test, expect } from '@playwright/test';
import { apiCtx, seedWorkspace, seedShellSession } from './seed';
import { openPage, expectFullyInViewport } from './helpers';

// Right panel tab strip at minimum width (260px). The 8 tab labels are wider
// than that, so the row must scroll horizontally — the expand/collapse buttons
// stay pinned and every tab stays reachable (regression: the buttons were
// pushed off the edge and the trailing tabs were unreachable).

let wsA = '';

test.beforeAll(async () => {
  const a = await apiCtx();
  wsA = await seedWorkspace(a.ctx, a.base);
  await seedShellSession(a.ctx, a.base, wsA);
  await a.ctx.dispose();
});

test.beforeEach(async ({ page }, testInfo) => {
  test.skip(testInfo.project.name !== 'desktop-browser', 'desktop-browser only');
  await page.addInitScript((wsId) => {
    localStorage.setItem('otto_workspace', wsId as string);
    localStorage.setItem('otto_firstrun_dismissed', '1');
    localStorage.setItem('otto_right_open', '1');
    localStorage.setItem('otto_right_width', '260');
  }, wsA);
});

test.describe('right panel tabs at min width', () => {
  test('buttons stay pinned and every tab is reachable', async ({ page }) => {
    await openPage(page, 'agents');
    // The panel renders only for an OPEN agent session — open the seeded one.
    await page.getByRole('button', { name: /E2E Shell/ }).first().click();
    const panel = page.locator('.rpanel');
    await expect(panel).toBeVisible();

    // The tab row genuinely overflows at 260px…
    const row = panel.locator('.rpanel-tabs');
    const overflows = await row.evaluate((el) => el.scrollWidth > el.clientWidth + 2);
    expect(overflows).toBe(true);

    // …but the expand + collapse buttons are still fully on screen.
    await expectFullyInViewport(page, panel.getByRole('button', { name: 'Expand panel' }));
    await expectFullyInViewport(page, panel.getByRole('button', { name: 'Collapse panel' }));

    // The last tab (API) is reachable: click scrolls it into view and selects it.
    const apiTab = panel.getByRole('tab', { name: 'API', exact: true });
    await apiTab.click();
    await expect(apiTab).toHaveAttribute('aria-selected', 'true');
    await expectFullyInViewport(page, apiTab);

    // And the first tab is still reachable after scrolling to the end.
    const gitTab = panel.getByRole('tab', { name: 'Git', exact: true });
    await gitTab.click();
    await expect(gitTab).toHaveAttribute('aria-selected', 'true');
  });
});
