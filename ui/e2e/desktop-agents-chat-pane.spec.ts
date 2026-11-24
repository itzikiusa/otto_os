import { test, expect, type APIRequestContext, type Page } from '@playwright/test';
import { apiCtx, seedWorkspace } from './seed';
import { expectFullyInViewport } from './helpers';
import { seedLongChat, writeChat } from './long-chat-fixture';

// A terminal agent session switched to Chat on Agents (surface 'agents'): the
// conversation is a flex column that scrolls inside its pane with the
// composer on screen, on the theme's own background (it was a block box that
// overflowed under the pane's dark terminal background); switching the pane
// to another session opens that one fresh (no search or window carried over).

let ctx: APIRequestContext;
let base = '';
const SHOTS = process.env.OTTO_E2E_SHOTS;

test.beforeEach(async ({}, info) => {
  test.skip(info.project.name !== 'desktop-browser', 'desktop-browser project only');
  ({ ctx, base } = await apiCtx());
});
test.afterEach(async () => {
  await ctx?.dispose();
});

async function seedChats(page: Page, n: number): Promise<string[]> {
  const wsId = await seedWorkspace(ctx, base);
  const ids: string[] = [];
  for (let i = 0; i < n; i++) ids.push(await seedLongChat(ctx, base, wsId, writeChat()));
  await page.addInitScript(({ id, sids }) => {
    localStorage.setItem('otto_workspace', id);
    localStorage.setItem('otto_firstrun_dismissed', '1');
    localStorage.setItem('otto_nav_all_ws', '0');
    localStorage.setItem('otto_view_mode', 'tabs');
    for (const s of sids) localStorage.setItem(`otto_pane_view:${s}`, 'chat');
  }, { id: wsId, sids: ids });
  return ids;
}

const paneOf = (page: Page, id: string) => page.locator(`[data-pane-key][data-session="${id}"]`).first();

test('a long terminal session in Chat scrolls inside its pane, composer on screen, on the theme background', async ({ page }, info) => {
  test.setTimeout(120_000);
  const [id] = await seedChats(page, 1);
  await page.goto(`/#/agents/${id}`);
  const pane = paneOf(page, id);
  const list = pane.locator('.conv-list');
  await list.locator('article.turn').last().waitFor({ state: 'visible', timeout: 30_000 });
  await expect.poll(() => list.locator('article.turn').count()).toBeGreaterThan(15);
  await expect(pane.locator('.conv')).toHaveCSS('display', 'flex');
  // The list scrolls itself and ends inside the pane: nothing is clipped off its bottom.
  await expect.poll(() => list.evaluate((el) => {
    const p = el.closest('[data-pane-key]')!.getBoundingClientRect();
    return el.scrollHeight > el.clientHeight + 4 && el.getBoundingClientRect().bottom <= p.bottom + 1;
  })).toBe(true);
  await expectFullyInViewport(page, pane.getByRole('textbox', { name: /^Message/ }), 'the composer');
  // ⋯ offers no terminal-only actions while the chat shows (they did nothing).
  await pane.getByRole('button', { name: /^More…/ }).click();
  await expect(page.getByRole('menuitem', { name: 'Rename' })).toBeVisible();
  await expect(page.getByRole('menuitem', { name: /^(Find in terminal|Redraw terminal)/ })).toHaveCount(0);
  await page.keyboard.press('Escape');
  for (const s of ['light', 'dark'] as const) {
    await page.emulateMedia({ colorScheme: s });
    await expect(page.locator('html')).toHaveAttribute('data-scheme', s);
    const [conv, bg] = await pane.locator('.conv').evaluate((el) => {
      const probe = document.createElement('div');
      probe.style.background = 'var(--bg)';
      document.body.append(probe);
      const want = getComputedStyle(probe).backgroundColor;
      probe.remove();
      return [getComputedStyle(el).backgroundColor, want];
    });
    expect(conv, `.conv paints --bg in ${s}`).toBe(bg);
    if (SHOTS) await page.screenshot({ path: `${SHOTS}/agents-chat-long-${info.project.name}-${s}.png`, animations: 'disabled' });
  }
});

test('switching the pane to another session opens its chat fresh: no search carried over', async ({ page }) => {
  test.setTimeout(120_000);
  const [a, b] = await seedChats(page, 2);
  await page.goto(`/#/agents/${a}`);
  const leaf = page.locator('[data-pane-key]').first();
  await leaf.locator('.conv-list article.turn').last().waitFor({ state: 'visible', timeout: 30_000 });
  await leaf.getByRole('button', { name: 'Search this conversation' }).click();
  await expect(leaf.getByRole('button', { name: 'Close search' })).toBeVisible();
  await page.getByTestId(`agents-row-${b}`).click();
  await expect(page.locator(`[data-pane-key][data-session="${b}"]`)).toBeVisible();
  const pb = paneOf(page, b);
  await pb.locator('.conv-list article.turn').last().waitFor({ state: 'visible', timeout: 30_000 });
  await expect(pb.locator(`.conv[data-session="${b}"]`)).toBeVisible();
  await expect(pb.getByRole('button', { name: 'Close search' })).toHaveCount(0);
});
