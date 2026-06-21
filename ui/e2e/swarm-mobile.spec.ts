import { test, expect, type Page } from '@playwright/test';
import { apiCtx, seedWorkspace, seedSwarm } from './seed';

// Durable mobile-layout coverage for the Agent Swarm page. Runs on every device
// project (phones + tablets). Seeds — via the API — a swarm with a real org tree
// (engineering-squad preset), a project, tasks across several Kanban columns with
// a dependency chain, and board posts. That makes the page's five views render
// with CONTENT so we can assert real layout behavior on a phone:
//
//   • the view switcher reaches all 5 tabs (it scrolls horizontally on a phone),
//   • each view (Org / Graph / Kanban / Runs / Feed) renders and fits the
//     viewport width (no element forces the page wider than the screen — the
//     Kanban columns + Graph canvas are the only INTENDED horizontal scrollers),
//   • the major regions scroll independently (org tree, kanban column body,
//     graph canvas pans, feed),
//   • on a phone the Swarms rail and the swarm header collapse to give the view
//     back its vertical room, and re-expand on tap.
//
// LIVE-DATA NOTE: the isolated test daemon never actually spawns agent CLIs, so
// there are no live RUNS (with sessions/tokens) or an open session panel. The
// Runs view + the graph's run-node interactions therefore show their empty/idle
// layout, which is what's asserted here. The org tree, kanban, graph DAG, and
// feed are all exercised against seeded data.

let workspaceId = '';
let swarmName = 'E2E Swarm';

test.beforeAll(async () => {
  const { ctx, base } = await apiCtx();
  workspaceId = await seedWorkspace(ctx, base);
  await seedSwarm(ctx, base, workspaceId);
  await ctx.dispose();
});

// Activate the seeded workspace (so swarms load) and close the nav drawer (which
// defaults open on a fresh phone profile and would cover the page).
test.beforeEach(async ({ page }) => {
  await page.addInitScript((wsId) => {
    localStorage.setItem('otto_workspace', wsId as string);
    localStorage.setItem('otto_rail_expanded', '0');
  }, workspaceId);
});

/** The document must not scroll horizontally — the page content fits the
 *  viewport. (Internal horizontal scrollers like the Kanban columns, the Graph
 *  canvas, and the tab switcher are exempt: we measure the PAGE itself.) */
async function expectFitsWidth(page: Page): Promise<void> {
  const o = await page.evaluate(() => {
    const de = document.documentElement;
    return { scrollW: de.scrollWidth, clientW: de.clientWidth, vw: window.innerWidth };
  });
  expect(o.scrollW, 'page must not overflow horizontally').toBeLessThanOrEqual(o.clientW + 1);
  expect(o.clientW).toBeLessThanOrEqual(o.vw + 1);
}

// The phone chrome (collapsible rail/header, scrollable switcher, vertical
// session split) is gated in CSS behind the ≤640px breakpoint — NOT the device
// type. So "iphone-landscape" (814px) actually renders the tablet/desktop layout.
// Gate phone-specific assertions on the ACTUAL viewport width, measured at
// runtime, exactly like the other mobile specs (db-mobile / brokers-mobile).
const PHONE_MAX = 640;
/** True when the configured viewport is phone-width (≤640px) — read from the
 *  project's viewport size so it's reliable BEFORE any navigation (used in
 *  test.skip()). innerWidth on about:blank can lag the project viewport. */
function phoneWidth(page: Page): boolean {
  return (page.viewportSize()?.width ?? 9999) <= PHONE_MAX;
}

/** Open the swarm page and the seeded swarm; returns once the header is shown. */
async function openSwarm(page: Page): Promise<void> {
  await page.goto('/#/swarm');
  await expect(page.locator('.swarm-page')).toBeVisible({ timeout: 30_000 });
  const item = page.locator('.swarm-item', { hasText: swarmName }).first();
  await expect(item).toBeVisible({ timeout: 20_000 });
  await item.click();
  await expect(page.locator('.swarm-head')).toBeVisible({ timeout: 20_000 });
}

// The five switcher labels. None is a substring of another, so a plain
// (substring) hasText match is unambiguous — and robust to the leading space the
// `<Icon/> {label}` markup puts in front of each label (an anchored /^X$/ regex
// would NOT match that raw text content).
const SWITCHER_LABELS = ['Org', 'Graph', 'Board', 'Runs', 'Feed'] as const;

/** Switch to one of the five views via the switcher tab. The switcher scrolls
 *  horizontally on a phone, so click() (which scrolls into view) is enough. */
async function switchView(page: Page, label: string): Promise<void> {
  await page.locator('.switcher .seg', { hasText: label }).first().click();
}

test('swarm page loads and the swarm list is usable', async ({ page }) => {
  await page.goto('/#/swarm');
  await expect(page.locator('.swarm-page')).toBeVisible({ timeout: 30_000 });
  await expect(page.locator('.swarm-item', { hasText: swarmName }).first()).toBeVisible({
    timeout: 20_000,
  });
  await expectFitsWidth(page);
});

test('opening a swarm shows the header + view switcher and fits the viewport', async ({ page }) => {
  await openSwarm(page);
  // Title + status pill render.
  await expect(page.locator('.swarm-head h2', { hasText: swarmName })).toBeVisible();
  await expect(page.locator('.status-pill').first()).toBeVisible();
  // The switcher and all five tabs exist.
  await expect(page.locator('.switcher')).toBeVisible();
  for (const t of SWITCHER_LABELS) {
    await expect(page.locator('.switcher .seg', { hasText: t })).toBeVisible();
  }
  await expectFitsWidth(page);
});

test('all five tabs are reachable (switcher scrolls on a phone) and each fits', async ({
  page,
}) => {
  await openSwarm(page);

  // At phone width the switcher must be a horizontal scroll container so the last
  // tab ("Feed") is reachable even on a 320px screen. (At ≥641px the desktop tab
  // row applies, so this is gated on the actual viewport width, not device type.)
  if (phoneWidth(page)) {
    const overflowX = await page
      .locator('.switcher')
      .evaluate((el) => getComputedStyle(el).overflowX);
    expect(['auto', 'scroll']).toContain(overflowX);
  }

  // Switch through every view; each must render its root and keep the page fitting.
  const cases: { tab: string; root: string }[] = [
    { tab: 'Org', root: '.tree' },
    { tab: 'Graph', root: '.graph-wrap' },
    { tab: 'Board', root: '.kanban' },
    { tab: 'Runs', root: '.runs' },
    { tab: 'Feed', root: '.board' },
  ];
  for (const c of cases) {
    await switchView(page, c.tab);
    await expect(page.locator(c.root)).toBeVisible({ timeout: 10_000 });
    await expectFitsWidth(page);
  }
});

test('Org view: the hierarchy renders and the tree scrolls independently', async ({ page }) => {
  await openSwarm(page);
  await switchView(page, 'Org');
  const tree = page.locator('.tree');
  await expect(tree).toBeVisible();
  // The preset seeds a multi-level org (a VP, a lead, several devs).
  const rows = page.locator('.tree .row');
  await expect(rows.first()).toBeVisible({ timeout: 10_000 });
  expect(await rows.count(), 'org tree should list multiple agents').toBeGreaterThan(3);
  // The tree is its own scroll container (overflow:auto).
  const overflow = await tree.evaluate((el) => getComputedStyle(el).overflow);
  expect(overflow).toMatch(/auto|scroll/);
  await expectFitsWidth(page);
});

test('Kanban view: columns scroll horizontally, each column body scrolls', async ({ page }) => {
  await openSwarm(page);
  await switchView(page, 'Board');
  await expect(page.locator('.kanban')).toBeVisible();

  // The columns strip is the intended horizontal scroller.
  const columns = page.locator('.kanban .columns');
  await expect(columns).toBeVisible();
  const colOverflowX = await columns.evaluate((el) => getComputedStyle(el).overflowX);
  expect(['auto', 'scroll']).toContain(colOverflowX);
  // It actually overflows (more columns than fit) and can scroll on a phone.
  const colScroll = await columns.evaluate((el) => {
    const before = el.scrollLeft;
    el.scrollLeft = 600;
    const after = el.scrollLeft;
    return { scrollable: el.scrollWidth > el.clientWidth, moved: after > before };
  });
  expect(colScroll.scrollable, 'kanban columns must overflow horizontally').toBe(true);
  expect(colScroll.moved, 'kanban columns must scroll horizontally').toBe(true);

  // Cards render (seeded tasks) and each column body is its own scroll container.
  await expect(page.locator('.kanban .card').first()).toBeVisible({ timeout: 10_000 });
  const bodyOverflowY = await page
    .locator('.kanban .col-body')
    .first()
    .evaluate((el) => getComputedStyle(el).overflowY);
  expect(['auto', 'scroll']).toContain(bodyOverflowY);

  // The PAGE still fits (the columns scroll internally, not the document).
  await expectFitsWidth(page);
});

test('Graph view: the DAG renders and the canvas pans/fits on a phone', async ({ page }) => {
  await openSwarm(page);
  await switchView(page, 'Graph');
  await expect(page.locator('.graph-wrap')).toBeVisible();

  // Seeded tasks + dependency edges → nodes render.
  await expect(page.locator('.graph-wrap .node').first()).toBeVisible({ timeout: 10_000 });
  expect(await page.locator('.graph-wrap .node').count()).toBeGreaterThan(2);

  // The canvas is the pan surface (touch-action:none so drag-to-pan works), and
  // the graph fits the page width (auto-fit on a phone scales it down).
  const touch = await page
    .locator('.graph-wrap .canvas')
    .evaluate((el) => getComputedStyle(el).touchAction);
  expect(touch).toBe('none');

  // The fit control re-centers without error.
  await page.locator('.graph-wrap .controls [aria-label="fit"]').click();
  await expect(page.locator('.graph-wrap .node').first()).toBeVisible();

  // The page never overflows horizontally — the graph canvas clips/pans inside
  // its own bounds at every width.
  await expectFitsWidth(page);
});

test('Runs view: filters + (empty/idle) list render and fit', async ({ page }) => {
  await openSwarm(page);
  await switchView(page, 'Runs');
  await expect(page.locator('.runs')).toBeVisible();
  // Filter controls render (assignee/project selects + status chips).
  await expect(page.locator('.runs .filters')).toBeVisible();
  await expect(page.locator('.runs .chip', { hasText: 'all' }).first()).toBeVisible();
  // No live runs in the test daemon → the empty state shows. The view must fit
  // either way (empty state OR a populated table).
  await expectFitsWidth(page);
});

test('Feed view: the board renders posts + composer and fits', async ({ page }) => {
  await openSwarm(page);
  await switchView(page, 'Feed');
  await expect(page.locator('.board')).toBeVisible();
  // Seeded board posts render in the scrollable feed.
  const feed = page.locator('.board .feed');
  await expect(feed).toBeVisible();
  await expect(page.locator('.board .msg').first()).toBeVisible({ timeout: 10_000 });
  const feedOverflowY = await feed.evaluate((el) => getComputedStyle(el).overflowY);
  expect(['auto', 'scroll']).toContain(feedOverflowY);
  // The composer (kind select + input + Post) stays reachable.
  await expect(page.locator('.board .composer')).toBeVisible();
  await expectFitsWidth(page);
});

// --- Phone-only chrome collapse -------------------------------------------
// The Swarms rail and the swarm header collapse on a phone to free vertical
// space for the chosen view; both re-expand on tap. Gated behind the ≤640px
// layout, so this only runs on phone-width projects.
test('phone: the Swarms rail collapses when a swarm opens and re-expands on tap', async ({
  page,
}) => {
  test.skip(!(phoneWidth(page)), 'rail collapse is a phone-width (≤640px) layout');
  await openSwarm(page);

  // After opening, the rail auto-collapses (list hidden, current name shown).
  await expect(page.locator('.rail.collapsed')).toBeVisible();
  await expect(page.locator('.rail .rail-current')).toContainText(swarmName);

  // The rail list is hidden while collapsed.
  await expect(page.locator('.rail .swarm-item')).toBeHidden();

  // Tap the rail header → list re-appears.
  await page.locator('.rail-toggle').click();
  await expect(page.locator('.rail .swarm-item', { hasText: swarmName }).first()).toBeVisible();

  // Tap again → collapses.
  await page.locator('.rail-toggle').click();
  await expect(page.locator('.rail .swarm-item')).toBeHidden();
});

test('phone: the swarm header controls collapse and re-expand on tap', async ({ page }) => {
  test.skip(!(phoneWidth(page)), 'header collapse is a phone-width (≤640px) layout');
  await openSwarm(page);

  // Header starts collapsed: controls hidden, title still visible.
  await expect(page.locator('.swarm-head.head-collapsed')).toBeVisible();
  await expect(page.locator('.swarm-head h2', { hasText: swarmName })).toBeVisible();
  await expect(page.locator('.swarm-head .head-controls')).toBeHidden();

  // Tap the header toggle → controls (parallel cap + lifecycle/recruit/etc) show.
  await page.locator('.swarm-head .head-toggle').click();
  await expect(page.locator('.swarm-head .head-controls')).toBeVisible();
  await expect(page.locator('.swarm-head #cap')).toBeVisible();

  // The expanded controls still fit the viewport width (they wrap, not clip).
  await expectFitsWidth(page);

  // Tap again → collapses.
  await page.locator('.swarm-head .head-toggle').click();
  await expect(page.locator('.swarm-head .head-controls')).toBeHidden();
});

// --- Wide layout keeps the desktop two-pane chrome ------------------------
// At ≥641px (tablet portrait/landscape, iphone landscape, desktop) the page
// keeps the persistent sidebar + always-open header — none of the phone
// collapse classes apply.
test('wide (≥641px): keeps the persistent rail (no phone collapse) and fits', async ({ page }) => {
  test.skip(phoneWidth(page), 'wide-layout-only: desktop two-pane chrome');
  await openSwarm(page);
  // The rail stays a plain sidebar — no collapse class, list visible.
  await expect(page.locator('.rail.collapsed')).toHaveCount(0);
  await expect(page.locator('.rail .swarm-item', { hasText: swarmName }).first()).toBeVisible();
  // Header controls are always shown (no head-collapsed class).
  await expect(page.locator('.swarm-head.head-collapsed')).toHaveCount(0);
  await expectFitsWidth(page);
});
