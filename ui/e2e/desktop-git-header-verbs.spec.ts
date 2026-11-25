import { test, expect, type Locator, type Page } from '@playwright/test';
import { EXPECT, VERBS, VERB_STATES, seedVerbRepos, type VerbState } from './git-header-verbs-fixture';
import { expectNoHorizontalOverflow } from './helpers';

// The everyday verbs stay one visible click away (the owner's rule: "pull,
// push, stash, pop — they always should be visible"). 0.5.0 moved all but
// Fetch into the header's ⋯. On four real repositories (local bare remotes,
// a real-length branch name, four repo tabs open) the header shows Fetch ·
// Pull · Push · Stash · Pop as labelled buttons in one row, each enabled only
// when it can do something; a narrow window turns labels into icons
// (Stash, Pop, Fetch first) and never moves them into ⋯.

test.use({ serviceWorkers: 'block' });
const SHOTS = process.env.OTTO_E2E_SHOTS;
let ids: Record<VerbState, string>;

test.beforeAll(async () => {
  ids = await seedVerbRepos(`hv${Date.now().toString(36)}`);
});

const header = (page: Page): Locator => page.locator('.git-header .ph-actions');
const verb = (page: Page, name: string): Locator => header(page).getByRole('button', { name: new RegExp(`^${name}\\b`) }).first();

async function open(page: Page, state: VerbState, view = 'graph'): Promise<void> {
  // Every repo open as a tab (the strip a daily user has), this one active.
  await page.addInitScript(() => localStorage.setItem('otto_firstrun_dismissed', '1'));
  await page.goto(`/#/git/${ids[state]}/${view}`);
  await expect(page.locator('.gitpage')).toBeVisible({ timeout: 30_000 });
  await expect(page.locator('.git-header .git-views')).toBeVisible({ timeout: 25_000 });
}

test('the five verbs are visible, labelled and enabled by state at 1440', async ({ page }) => {
  test.setTimeout(120_000);
  await page.setViewportSize({ width: 1440, height: 900 });
  // Open all four as tabs first.
  for (const s of VERB_STATES) await open(page, s);
  for (const s of VERB_STATES) {
    await open(page, s);
    for (const v of VERBS) await expect(verb(page, v), `${s}: ${v} visible`).toBeVisible({ timeout: 20_000 });
    const xs = await Promise.all(VERBS.map((v) => verb(page, v).boundingBox()));
    for (const [i, v] of VERBS.entries()) {
      const b = verb(page, v);
      // In Fetch · Pull · Push · Stash · Pop order; Pull and Push always say their word.
      if (v === 'Pull' || v === 'Push') await expect(b, `${s}: ${v} labelled`).not.toHaveClass(/icon-only/);
      if (i > 0) expect(xs[i]!.x, `${v} after ${VERBS[i - 1]}`).toBeGreaterThan(xs[i - 1]!.x);
      const want = EXPECT[s][v];
      if (want.enabled) await expect(b, `${s}: ${v} enabled`).toBeEnabled({ timeout: 15_000 });
      else await expect(b, `${s}: ${v} disabled`).toBeDisabled({ timeout: 15_000 });
      if (!want.enabled) expect(await b.getAttribute('title'), `${s}: disabled ${v} says why`).toBeTruthy();
      if (want.text) await expect(b, `${s}: ${v} count`).toContainText(want.text);
    }
    // Push is the accent only with something to push and nothing to commit.
    await expect(verb(page, 'Push')).toHaveClass(s === 'ahead' ? /primary/ : /ghost/);
    // One row: nothing in ⋯, nothing scrolled off, no page scroll.
    await expect(header(page).locator('[data-ph-hidden]')).toHaveCount(0);
    const row = await page.locator('.git-header .ph-row').boundingBox();
    expect(Math.round(row!.height)).toBeLessThanOrEqual(46);
    expect(await page.locator('.git-header .ph-actions-wrap').evaluate((w) => w.scrollWidth - w.clientWidth)).toBeLessThanOrEqual(1);
    await expectNoHorizontalOverflow(page);
    // ⋯ keeps only the rare ones: no duplicates of the visible verbs.
    await page.locator('.git-header .ph-more').click();
    const menu = page.locator('.ctx-menu');
    await expect(menu.getByRole('menuitem', { name: /^New branch/ })).toBeVisible();
    await expect(menu.getByRole('menuitem', { name: /^(Pull|Push|Publish|Stash changes|Pop stash|Fetch)\b/ })).toHaveCount(0);
    await page.keyboard.press('Escape');
    if (SHOTS) {
      for (const scheme of ['light', 'dark'] as const) {
        await page.emulateMedia({ colorScheme: scheme });
        await page.screenshot({ path: `${SHOTS}/desktop-${s}-${scheme}.png` });
      }
      await page.emulateMedia({ colorScheme: 'light' });
    }
  }
});

test('Pull keeps its modes on the caret; Pop lists the stashes', async ({ page }) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  await open(page, 'behind');
  const menu = page.locator('.ctx-menu');
  await header(page).getByRole('button', { name: 'Pull options', exact: true }).click();
  await expect(menu.getByRole('menuitem', { name: /^Pull \(merge\)/ })).toContainText('default');
  await expect(menu.getByRole('menuitem', { name: /^Pull \(rebase\)/ })).toBeVisible();
  await expect(menu.getByRole('menuitem', { name: /^Pull \(fast-forward only\)/ })).toBeVisible();
  await page.keyboard.press('Escape');
  await header(page).getByRole('button', { name: 'Stashes', exact: true }).click();
  await expect(menu.getByRole('menuitem', { name: /wip 1/ })).toBeVisible();
  await expect(menu.getByRole('menuitem', { name: /wip 0/ })).toBeVisible();
  await menu.getByRole('menuitem', { name: /wip 1/ }).hover();
  await expect(menu.getByRole('menuitem', { name: 'Apply', exact: true })).toBeVisible();
  await expect(menu.getByRole('menuitem', { name: 'Drop…', exact: true })).toBeVisible();
  await page.keyboard.press('Escape');
});

test('narrow window: the views, then labels turn into icons (Stash, Pop, Fetch first), nothing moves into ⋯', async ({ page }) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  for (const s of VERB_STATES) await open(page, s);
  await open(page, 'behind');
  for (const w of [1440, 1280, 1100, 1030]) {
    await page.setViewportSize({ width: w, height: 900 });
    // Where the row can hold the verbs as icons it settles with nothing
    // scrolled off (labels give way first); narrower, the header scrolls.
    if (w >= 1280) await expect.poll(() => page.locator('.git-header .ph-actions-wrap').evaluate((el) => el.scrollWidth - el.clientWidth)).toBeLessThanOrEqual(1);
    else await expect(header(page).locator('.git-views.compact, .git-views-menu').first()).toBeVisible(); // the views give way before the verbs' labels (R2-001)
    for (const v of VERBS) await expect(verb(page, v), `${w}: ${v} visible`).toBeVisible();
    await expect(header(page).locator('[data-ph-hidden]')).toHaveCount(0);
    const labelled = await Promise.all(VERBS.map(async (v) => !/icon-only/.test((await verb(page, v).getAttribute('class')) ?? '')));
    const at = Object.fromEntries(VERBS.map((v, i) => [v, labelled[i]]));
    console.log('LABELS', w, JSON.stringify(at));
    // Collapse order: Stash, then Pop, then Fetch; Pull and Push keep their labels longest.
    if (!at.Pull || !at.Push) expect(at.Fetch || at.Stash || at.Pop, `${w}: Pull/Push lost labels first`).toBe(false);
    if (at.Stash) expect(at.Pop && at.Fetch, `${w}: Stash collapses first`).toBe(true);
    if (at.Pop) expect(at.Fetch, `${w}: Pop before Fetch`).toBe(true);
    const row = await page.locator('.git-header .ph-row').boundingBox();
    expect(Math.round(row!.height)).toBeLessThanOrEqual(46);
    await expectNoHorizontalOverflow(page);
  }
});

