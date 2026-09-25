import { test, expect } from '@playwright/test';
import { apiCtx, seedWorkspace, seedShellSession } from './seed';
import { openPage, expectFullyInViewport } from './helpers';

// Right panel tab strip at minimum width (260px). The 9 tab labels are wider
// than that, so the tabs that don't fit fold into a "More panels" menu — the
// expand/collapse buttons stay pinned, no label is cut at the edge, and every
// tab stays reachable (regression: the row scrolled and the edge tab was cut
// mid-word; before that, the buttons were pushed off the edge).

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

    // The tabs don't all fit at 260px → some fold into "More panels"…
    const more = panel.getByRole('button', { name: 'More panels' });
    await expect(more).toBeVisible();
    // …and no visible tab is clipped by the row.
    const row = panel.locator('.rpanel-tabs');
    const clipped = await row.evaluate((el) => {
      const r = el.getBoundingClientRect();
      return Array.from(el.querySelectorAll<HTMLElement>('.rtab'))
        .filter((t) => t.offsetWidth > 0)
        .some((t) => {
          const b = t.getBoundingClientRect();
          return b.left < r.left - 0.5 || b.right > r.right + 0.5;
        });
    });
    expect(clipped).toBe(false);

    // …but the expand + collapse buttons are still fully on screen.
    await expectFullyInViewport(page, panel.getByRole('button', { name: 'Expand panel' }));
    await expectFullyInViewport(page, panel.getByRole('button', { name: 'Collapse panel' }));

    // The last tab (API) is reachable from the menu: it becomes the active,
    // visible tab.
    await more.click();
    await page.locator('.ctx-menu').getByRole('menuitem', { name: 'API' }).click();
    const apiTab = panel.getByRole('tab', { name: 'API', exact: true });
    await expect(apiTab).toHaveAttribute('aria-selected', 'true');
    await expectFullyInViewport(page, apiTab);

    // And the first tab is still reachable.
    const gitTab = panel.getByRole('tab', { name: 'Git', exact: true });
    await gitTab.click();
    await expect(gitTab).toHaveAttribute('aria-selected', 'true');
  });
});
