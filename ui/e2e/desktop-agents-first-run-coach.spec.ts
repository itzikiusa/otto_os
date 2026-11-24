import { test, expect, type APIRequestContext, type Page } from '@playwright/test';
import { apiCtx, seedWorkspace, tempDir } from './seed';

// The first-run coach ("Let's launch your first agent") is for a first-timer
// only: it never flashes while the session lists load on a cold boot, and
// someone with structured threads only does not get it.

let ctx: APIRequestContext;
let base = '';
const COACH = 'Let’s launch your first agent';

test.beforeEach(async ({}, info) => {
  test.skip(info.project.name !== 'desktop-browser', 'desktop-browser project only');
  ({ ctx, base } = await apiCtx());
});
test.afterEach(async () => {
  await ctx?.dispose();
});

/** Fresh browser (coach not dismissed) on `wsId`; records whether the coach EVER showed. */
async function freshBrowser(page: Page, wsId: string, allWs: boolean): Promise<void> {
  await page.addInitScript(({ id, all, coach }) => {
    localStorage.setItem('otto_workspace', id);
    localStorage.removeItem('otto_firstrun_dismissed');
    localStorage.setItem('otto_nav_all_ws', all ? '1' : '0');
    new MutationObserver(() => {
      if (document.body?.innerText.includes(coach)) (window as unknown as { coachSeen: boolean }).coachSeen = true;
    }).observe(document, { childList: true, subtree: true, characterData: true });
  }, { id: wsId, all: allWs, coach: COACH });
}
const coachSeen = (page: Page) => page.evaluate(() => (window as unknown as { coachSeen?: boolean }).coachSeen === true);
const slow = (page: Page, re: RegExp) => page.route(re, async (route) => {
  if (route.request().method() !== 'GET') return route.fallback();
  await new Promise((r) => setTimeout(r, 1_200));
  await route.fallback();
});

test('a cold boot with sessions only in another workspace never flashes the coach while the lists load', async ({ page }) => {
  const empty = await seedWorkspace(ctx, base, 'Coach empty WS');
  const other = await seedWorkspace(ctx, base, 'Coach other WS');
  const r = await ctx.post(`${base}/api/v1/workspaces/${other}/sessions`, { data: { kind: 'agent', provider: 'shell', title: 'Rijkaard', cwd: tempDir('otto-coach-'), meta: { origin: 'e2e' } } });
  expect(r.ok(), await r.text()).toBeTruthy();
  const id = (await r.json()).id as string;
  await slow(page, /\/api\/v1\/workspaces$/);
  await slow(page, /\/api\/v1\/sessions\?archived=false&foreground=true$/);
  await freshBrowser(page, empty, true);
  await page.goto('/#/agents');
  await expect(page.getByTestId(`agents-row-${id}`)).toBeVisible({ timeout: 30_000 });
  await page.waitForTimeout(500); // ui-guards: allow — proves an absence (the coach never shows)
  expect(await coachSeen(page), 'the coach flashed while the lists loaded').toBe(false);
});

test('someone with structured threads only gets no coach', async ({ page }) => {
  const wsId = await seedWorkspace(ctx, base, 'Coach threads WS');
  const t = await ctx.post(`${base}/api/v1/workspaces/${wsId}/runtime-sessions`, { data: { provider: 'codex', cwd: tempDir('otto-coach-t-'), title: 'Seedorf chat' } });
  expect(t.ok(), await t.text()).toBeTruthy();
  const threadId = (await t.json()).id as string;
  try {
    await freshBrowser(page, wsId, false);
    await page.goto('/#/agents');
    await expect(page.getByTestId(`agents-row-${threadId}`)).toBeVisible({ timeout: 30_000 });
    await page.waitForTimeout(500); // ui-guards: allow — proves an absence (the coach never shows)
    expect(await coachSeen(page)).toBe(false);
  } finally {
    await ctx.post(`${base}/api/v1/runtime-sessions/${threadId}/close`).catch(() => {});
  }
});

test('a first-timer still gets the coach once the lists answered', async ({ page }) => {
  const wsId = await seedWorkspace(ctx, base, 'Coach first WS');
  // No sessions or threads anywhere for this person (other specs' rows are not theirs).
  await page.route(/\/api\/v1\/runtime-sessions$/, (route) => route.request().method() === 'GET' ? route.fulfill({ json: [] }) : route.fallback());
  await page.route(/\/api\/v1\/workspaces\/scratch\/sessions(\?.*)?$/, (route) => route.request().method() === 'GET' ? route.fulfill({ json: [] }) : route.fallback());
  await freshBrowser(page, wsId, false);
  await page.goto('/#/agents');
  await expect(page.getByText(COACH)).toBeVisible({ timeout: 30_000 });
});
