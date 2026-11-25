import { test, expect, type Page } from '@playwright/test';
import { bootWriteIde, openTreeFile } from './ide-write-fixtures';

// Returning to the IDE mounts a new editor. It used to start from placeholder gutters and then
// reconfigure the full IDE extension set, the theme and every prop a microtask later: two
// configuration rebuilds per mount, ~20 ms of the WebKit page transition, and each mount added
// a new copy of a theme's rules to the page. A new editor now starts with its final configuration.

// CodeMirror's rules only (scoped by its generated "ͼ" classes): pages load their own CSS lazily.
const cssRules = (page: Page): Promise<number> => page.evaluate(() =>
  [...Array.from(document.styleSheets), ...document.adoptedStyleSheets].reduce((n, s) => n + Array.from(s.cssRules).filter((r) => r.cssText.includes('ͼ')).length, 0));

/** Count the transactions that reconfigure the live editor from now on. */
async function countUpdates(page: Page): Promise<void> {
  await page.evaluate(() => {
    const el = document.querySelector('.ide-editor-area .cm-content') as unknown as { cmTile: { root: { view: object } } };
    const proto = Object.getPrototypeOf(el.cmTile.root.view) as { update: (trs: { reconfigured: boolean; effects: { value?: { extension?: unknown } }[] }[]) => void };
    const w = window as unknown as { __cmReconf: number };
    const orig = proto.update;
    proto.update = function (this: unknown, trs: { reconfigured: boolean; effects: { value?: { extension?: unknown } }[] }[]) {
      // A language pack that finishes loading swaps the language compartment: not counted here.
      const isLang = (x: unknown): boolean => Array.isArray(x) ? x.some(isLang) : !!x && typeof x === 'object' && 'language' in x;
      w.__cmReconf += trs.filter((t) => t.reconfigured && !t.effects.some((e) => isLang(e.value?.extension))).length;
      return orig.call(this, trs);
    };
  });
}

test('returning to the IDE mounts the editor without reconfiguring it', async ({ page }) => {
  test.setTimeout(90_000);
  await page.setViewportSize({ width: 1440, height: 900 });
  await bootWriteIde(page);
  await openTreeFile(page, 'cmd/main.go');
  const area = page.locator('.ide-editor-area');
  await expect(area.locator('[class*="cm-syn-"]').first()).toBeVisible({ timeout: 15_000 });
  const leaveAndReturn = async (): Promise<void> => {
    await page.evaluate(() => { location.hash = '#/home'; });
    await expect(area).toHaveCount(0);
    await page.evaluate(() => { (window as unknown as { __cmReconf: number }).__cmReconf = 0; location.hash = '#/ide'; });
    await expect(area.locator('.cm-line').first()).toBeVisible({ timeout: 15_000 });
    await page.waitForTimeout(1000); // ui-guards: allow (proves nothing reconfigures after the mount)
  };
  await leaveAndReturn(); // Home's own styles load on its first visit
  const rules = await cssRules(page);
  await countUpdates(page);
  await leaveAndReturn();

  expect(await page.evaluate(() => (window as unknown as { __cmReconf: number }).__cmReconf)).toBe(0);
  // No editor theme is built anew per mount: the stylesheet the page matches against stays put.
  expect(await cssRules(page)).toBe(rules);
  // The first state already carries the IDE extensions: gutters, semantic colours.
  await expect(area.locator('.cm-ide-bp-gutter')).toHaveCount(1);
  await expect(area.locator('[class*="cm-syn-"]').first()).toBeVisible();
});
