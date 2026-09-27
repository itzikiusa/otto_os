import { test, expect, type Page } from '@playwright/test';
import { apiCtx, seedWorkspace } from './seed';

// ─────────────────────────────────────────────────────────────────────────────
// Shell main-thread multipliers (perf backlog B3), behaviour kept identical:
//  - ⌘F find-in-page paints matches from StaticRanges (the document no longer
//    tracks a live Range per match), caps at 5000 ("N / 5000+"), re-searches
//    when the current match's text was re-rendered away, and drops everything
//    on close;
//  - resizer drags carry the cursor on a full-window overlay (never
//    body.style) and persist the width once, on release.
// ─────────────────────────────────────────────────────────────────────────────

let wsId = '';

test.beforeAll(async () => {
  const { ctx, base } = await apiCtx();
  wsId = await seedWorkspace(ctx, base);
  await ctx.dispose();
});

test.beforeEach(async ({ page }, info) => {
  test.skip(info.project.name !== 'desktop-browser', 'desktop-browser project only');
  await page.addInitScript((id) => {
    localStorage.setItem('otto_workspace', id as string);
    localStorage.setItem('otto_firstrun_dismissed', '1');
  }, wsId);
  await page.goto('/#/settings/appearance');
  await expect(page.locator('.content').first()).toBeVisible();
});

/** Append `n` spans holding `word` to the page content (what ⌘F walks). */
async function seedText(page: Page, word: string, n: number): Promise<void> {
  await page.evaluate(
    ({ word, n }) => {
      const host = document.createElement('div');
      host.id = 'find-seed';
      for (let i = 0; i < n; i++) {
        const s = document.createElement('span');
        s.textContent = `${word} `;
        host.appendChild(s);
      }
      document.querySelector('.content')!.appendChild(host);
    },
    { word, n },
  );
}

const bar = (page: Page) => page.locator('.otto-find-bar');
const count = (page: Page) => bar(page).locator('.find-count');

async function openFind(page: Page): Promise<void> {
  // Nothing focused → ⌘F is the page-wide overlay (not a component's own find).
  await page.evaluate(() => (document.activeElement as HTMLElement | null)?.blur());
  await page.keyboard.press('ControlOrMeta+f');
  await expect(bar(page)).toBeVisible();
}

test('find-in-page highlights with static ranges, steps, recovers from re-render, clears on close', async ({ page }) => {
  await seedText(page, 'zebrafind', 12);
  await openFind(page);
  await page.keyboard.type('zebrafind');
  await expect(count(page)).toHaveText('1 / 12');
  const kinds = await page.evaluate(() => {
    const h = (CSS as unknown as { highlights: Map<string, Set<AbstractRange>> }).highlights.get('otto-find');
    return h ? [...h].map((r) => r.constructor.name) : [];
  });
  expect(kinds).toHaveLength(12);
  expect(new Set(kinds)).toEqual(new Set(['StaticRange']));

  await page.keyboard.press('Enter');
  await expect(count(page)).toHaveText('2 / 12');
  await page.keyboard.press('Shift+Enter');
  await expect(count(page)).toHaveText('1 / 12');

  // The next match's text node is replaced (a re-render): stepping re-searches.
  await page.evaluate(() => {
    const spans = document.querySelectorAll('#find-seed span');
    spans[1].textContent = 'gone ';
  });
  await page.keyboard.press('Enter');
  await expect(count(page)).toHaveText('2 / 11');

  await page.keyboard.press('Escape');
  await expect(bar(page)).toHaveCount(0);
  const left = await page.evaluate(() => {
    const hl = (CSS as unknown as { highlights: Map<string, unknown> }).highlights;
    return [hl.has('otto-find'), hl.has('otto-find-current')];
  });
  expect(left).toEqual([false, false]);
});

test('find-in-page caps at 5000 matches', async ({ page }) => {
  await seedText(page, 'capword', 6000);
  await openFind(page);
  await page.keyboard.type('capword');
  await expect(count(page)).toHaveText('1 / 5000+');
});

test('sidebar resize: overlay cursor while dragging, body untouched, width persisted on release', async ({ page }) => {
  const handle = page.locator('.rail-resize');
  const box = (await handle.boundingBox())!;
  const before = await page.evaluate(() => document.querySelector<HTMLElement>('.navigator')!.getBoundingClientRect().width);
  await page.mouse.move(box.x + box.width / 2, box.y + 200);
  await page.mouse.down();
  await page.mouse.move(box.x + box.width / 2 + 60, box.y + 200, { steps: 8 });
  await expect(page.getByTestId('drag-overlay')).toHaveCount(1);
  const during = await page.evaluate(() => ({
    bodyCursor: document.body.style.cursor,
    overlayCursor: getComputedStyle(document.querySelector('[data-testid="drag-overlay"]')!).cursor,
    stored: localStorage.getItem('otto_rail_width'),
  }));
  expect(during.bodyCursor).toBe('');
  expect(during.overlayCursor).toBe('col-resize');
  await page.mouse.up();
  await expect(page.getByTestId('drag-overlay')).toHaveCount(0);
  const after = await page.evaluate(() => ({
    width: document.querySelector<HTMLElement>('.navigator')!.getBoundingClientRect().width,
    stored: localStorage.getItem('otto_rail_width'),
  }));
  expect(after.width).toBeGreaterThan(before + 30);
  expect(Number(after.stored)).toBe(Math.round(after.width));
  expect(during.stored).not.toBe(after.stored);
});
