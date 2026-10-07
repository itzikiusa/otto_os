import { test, expect, type APIRequestContext, type Page } from '@playwright/test';
import { apiCtx, seedWorkspace } from './seed';
import { expectFullyInViewport, expectNoHorizontalOverflow } from './helpers';

// ─────────────────────────────────────────────────────────────────────────────
// The session pane header stays usable at 1, 2 and 6 tiled panes (desktop).
//
// The header used to pack status, title, provider, idle countdown, cwd, a
// three-way Terminal · Chat · Split control, font −/size/+, copy, zoom, UI
// control, restart and ⋯ into one row, so at 2 panes a long title read
// "i have some perform…" and at 6 "i h…". It is now one compact row adapted to
// the pane width (lib/paneHeader.ts tiers); the title gets the space and the
// pane controls moved into ⋯. This spec pins that down:
//   • the title shows at least TITLE_CHARS characters untruncated,
//   • no header overflows, wraps or changes height,
//   • ⋯ holds the moved actions (font, copy-on-select, restart).
// Light + dark screenshots at each count go to $OTTO_HEADER_SHOTS_DIR when set
// (else the test output dir).
// ─────────────────────────────────────────────────────────────────────────────

const TITLE = 'i have some performance issues with the tiled grid when many agents run';
/** Characters of the title that must be visible (not behind the ellipsis). */
const TITLE_CHARS = 24;
const HEADER_H = 30;
const SHOTS = process.env.OTTO_HEADER_SHOTS_DIR;

let ctx: APIRequestContext;
let base = '';

test.beforeAll(async () => {
  const c = await apiCtx();
  ctx = c.ctx;
  base = c.base;
});

test.afterAll(async () => {
  await ctx?.dispose();
});

/** A fresh workspace holding `n` agent sessions (shell provider, cheap to spawn). */
async function seed(n: number): Promise<string> {
  const wsId = await seedWorkspace(ctx, base);
  for (let i = 0; i < n; i++) {
    const r = await ctx.post(`${base}/api/v1/workspaces/${wsId}/sessions`, {
      data: {
        kind: 'agent',
        provider: 'shell',
        title: i === 0 ? TITLE : `${TITLE} (${i + 1})`,
        cwd: '/tmp',
        meta: { origin: 'e2e' },
      },
    });
    if (!r.ok()) throw new Error(`seed session ${i} → ${r.status()} ${await r.text()}`);
  }
  return wsId;
}

/** Width the first `n` characters of the title need, vs the width it has. */
async function titleFit(page: Page, i: number, n: number): Promise<{ need: number; have: number; text: string }> {
  return page.locator('.pane-title').nth(i).evaluate((el, count) => {
    const node = el.firstChild;
    const text = el.textContent ?? '';
    const range = document.createRange();
    if (node) {
      range.setStart(node, 0);
      range.setEnd(node, Math.min(count, text.length));
    }
    const cs = getComputedStyle(el);
    const pad = parseFloat(cs.paddingInlineStart) + parseFloat(cs.paddingInlineEnd);
    return { need: range.getBoundingClientRect().width + pad, have: (el as HTMLElement).clientWidth, text };
  }, n);
}

for (const scheme of ['light', 'dark'] as const) {
  for (const count of [1, 2, 6]) {
    test(`header at ${count} tiled pane${count > 1 ? 's' : ''} (${scheme})`, async ({ page }, info) => {
      test.skip(info.project.name !== 'desktop-browser', 'desktop-browser project only');
      test.slow();
      const wsId = await seed(count);
      await page.setViewportSize({ width: 1440, height: 900 });
      await page.addInitScript(
        ({ id, scheme }) => {
          localStorage.setItem('otto_workspace', id);
          localStorage.setItem('otto_firstrun_dismissed', '1');
          localStorage.setItem('otto_nav_all_ws', '0');
          localStorage.setItem('otto_right_open', '0');
          localStorage.setItem('otto_theme', 'native');
          localStorage.setItem('otto_scheme', scheme);
          localStorage.setItem('otto_view_mode', 'tiled');
        },
        { id: wsId, scheme },
      );
      await page.goto('/#/agents');
      const heads = page.locator('.pane-head');
      await expect(heads).toHaveCount(count, { timeout: 30_000 });
      await expect(page.locator('.pane-title').first()).toBeVisible();
      // Let the terminals attach and the fonts settle before measuring.
      await page.waitForTimeout(800);

      for (let i = 0; i < count; i++) {
        const head = heads.nth(i);
        const box = (await head.boundingBox())!;
        // One row of constant height, nothing clipped.
        expect(box.height, `header ${i} height`).toBe(HEADER_H);
        const over = await head.evaluate((el) => el.scrollWidth - el.clientWidth);
        expect(over, `header ${i} horizontal overflow (px)`).toBeLessThanOrEqual(2);
        // Every visible control sits inside the header box (no wrap, no spill).
        const kids = await head.evaluate((el) =>
          [...el.children]
            .filter((c) => getComputedStyle(c).display !== 'none' && (c as HTMLElement).offsetWidth > 0)
            .map((c) => {
              const r = c.getBoundingClientRect();
              return { cls: c.className, top: r.top, bottom: r.bottom, left: r.left, right: r.right };
            }),
        );
        for (const k of kids) {
          expect(k.top, `${k.cls} top`).toBeGreaterThanOrEqual(box.y - 0.5);
          expect(k.bottom, `${k.cls} bottom`).toBeLessThanOrEqual(box.y + box.height + 0.5);
          expect(k.right, `${k.cls} right`).toBeLessThanOrEqual(box.x + box.width + 0.5);
        }
        // The title keeps TITLE_CHARS readable characters.
        const fit = await titleFit(page, i, TITLE_CHARS);
        expect(fit.text.startsWith(TITLE)).toBe(true);
        expect(fit.have, `title ${i} width ${fit.have}px vs ${fit.need}px for ${TITLE_CHARS} chars`).toBeGreaterThanOrEqual(fit.need - 0.5);
        // The title's tooltip carries the whole text.
        await expect(page.locator('.pane-title').nth(i)).toHaveAttribute('title', new RegExp(`^${TITLE.slice(0, 20)}`));
        // Status + title + ⋯ are always there; the view switch is reachable inline.
        await expect(head.locator('[role="img"]').first()).toBeVisible();
        await expect(head.locator('button[title="More…"]')).toBeVisible();
        const inlineSwitch = head.locator('[role="tablist"][aria-label="Session view"]:visible, [data-view-toggle]:visible');
        await expect(inlineSwitch).toHaveCount(1);
        const buttons = head.locator('button:visible');
        const nb = await buttons.count();
        // Chrome stays lean: at most toggle(2 tabs) + details + zoom + ⋯ (+ grip).
        expect(nb, `header ${i} visible buttons`).toBeLessThanOrEqual(6);
        for (let b = 0; b < nb; b++) await expectFullyInViewport(page, buttons.nth(b), `header ${i} button ${b}`);
      }
      await expectNoHorizontalOverflow(page);

      // The tiers the widths should land in at 1440×900 with the right panel closed.
      const tiers = await heads.evaluateAll((els) => els.map((el) => el.getAttribute('data-tier')));
      if (count === 1) expect(tiers).toEqual(['full']);
      if (count === 2) expect(tiers.every((t) => t === 'compact' || t === 'full')).toBe(true);
      if (count === 6) expect(tiers.every((t) => t === 'minimal' || t === 'compact')).toBe(true);

      // Focus the first pane: the active one reads at full contrast, the others
      // step back (dimmed title) — no extra chrome.
      await heads.first().locator('.pane-title').click();
      await expect(page.locator('.pane.current')).toHaveCount(1);
      await expect(page.locator('.pane').first()).toHaveClass(/\bcurrent\b/);
      if (count > 1) {
        // The current class changes before the title's colour transition ends.
        // Assert the visible contrast once it updates, not its starting colours.
        await expect(async () => {
          const colours = await page.locator('.pane-title').evaluateAll((els) => els.map((el) => getComputedStyle(el).color));
          expect(new Set(colours.slice(1)).size, 'inactive titles share one colour').toBe(1);
          expect(colours[0], 'the active title differs from the inactive ones').not.toBe(colours[1]);
        }).toPass({ timeout: 10_000 });
      }
      await page.mouse.move(0, 450);

      const shot = `${count}-pane${count > 1 ? 's' : ''}-${scheme}.png`;
      await page.screenshot({ path: SHOTS ? `${SHOTS}/${shot}` : info.outputPath(shot), animations: 'disabled' });

      // ⋯ carries the actions that moved out of the header.
      await heads.first().locator('button[title="More…"]').click();
      const menu = page.locator('.ctx-menu');
      await expect(menu).toBeVisible();
      await expectFullyInViewport(page, menu, 'pane ⋯ menu');
      await expect(menu.getByRole('menuitem', { name: /^Terminal font larger/ })).toBeVisible();
      await expect(menu.getByRole('menuitem', { name: /^Terminal font smaller/ })).toBeVisible();
      await expect(menu.getByRole('menuitem', { name: /^Reset terminal font/ })).toBeVisible();
      await expect(menu.getByRole('menuitemcheckbox', { name: /^Copy on select/ })).toBeVisible();
      await expect(menu.getByRole('menuitem', { name: 'Restart session' })).toBeVisible();
      if (count === 1) {
        await page.screenshot({ path: SHOTS ? `${SHOTS}/1-pane-menu-${scheme}.png` : info.outputPath(`1-pane-menu-${scheme}.png`), animations: 'disabled' });
      }
      await page.keyboard.press('Escape');
      await expect(menu).toBeHidden();

      // The details chip opens the details as a menu (provider + folder).
      const chip = heads.first().locator('[data-testid="pane-details"]');
      if (await chip.isVisible()) {
        await expect(chip).toHaveAttribute('title', /Agent: shell/);
        await chip.click();
        await expect(menu.getByRole('menuitem', { name: /^Folder: \/tmp/ })).toBeDisabled();
        await expect(menu.getByRole('menuitem', { name: 'Copy folder path' })).toBeVisible();
        await page.keyboard.press('Escape');
      }
    });
  }
}
