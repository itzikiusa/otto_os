import { test, expect, type APIRequestContext, type Page } from '@playwright/test';
import { apiCtx, seedWorkspace } from './seed';

// M5 (design review p2): a pane / window resize makes every shell redraw its
// prompt, and that redraw used to read as output — each tiled shell flipped to
// "Working" and then "Done" (unread, sorted first) after a mere resize. Four
// idle shells tiled, the window resized a few times: every row still says Idle,
// and the daemon never stored "working" for any of them.

let ctx: APIRequestContext;
let base = '';
let wsId = '';
const TITLES = ['Baresi', 'Costacurta', 'Tassotti', 'Albertini'];

test.beforeEach(async ({ page }, info) => {
  test.skip(info.project.name !== 'desktop-browser', 'desktop-browser project only');
  ({ ctx, base } = await apiCtx());
  wsId = await seedWorkspace(ctx, base);
  for (const title of TITLES) {
    const r = await ctx.post(`${base}/api/v1/workspaces/${wsId}/sessions`, {
      data: { kind: 'agent', provider: 'shell', title, cwd: '/tmp', meta: { origin: 'e2e' } },
    });
    if (!r.ok()) throw new Error(`seed ${title} → ${r.status()} ${await r.text()}`);
  }
  await page.addInitScript((id) => {
    if (sessionStorage.getItem('e2e_seeded')) return;
    sessionStorage.setItem('e2e_seeded', '1');
    localStorage.setItem('otto_workspace', id as string);
    localStorage.setItem('otto_firstrun_dismissed', '1');
    localStorage.setItem('otto_nav_all_ws', '0');
    localStorage.setItem('otto_agents_fold_nav', '1');
  }, wsId);
});

test.afterEach(async () => {
  await ctx?.dispose();
});

const words = (page: Page) => page.locator(`.ai-row[data-workspace="${wsId}"] .tr-status`).allInnerTexts();
async function stored(): Promise<string[]> {
  const r = await ctx.get(`${base}/api/v1/workspaces/${wsId}/sessions`);
  const body = await r.json();
  return (Array.isArray(body) ? body : body.sessions ?? body.items ?? []).map((s: { status: string }) => s.status);
}

test('a resize redraw is not work: tiled idle shells stay Idle', async ({ page }) => {
  test.setTimeout(150_000);
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto('/#/agents');
  await expect(page.locator(`.ai-row[data-workspace="${wsId}"]`)).toHaveCount(4, { timeout: 30_000 });
  await page.getByTestId('agents-select-toggle').click();
  for (const t of TITLES) await page.getByLabel(`Select ${t}`).check();
  await page.getByTestId('agents-tile-selected').click();
  await expect(page.locator('[data-tile-id]')).toHaveCount(4, { timeout: 30_000 });
  // Each shell repaints its whole screen on SIGWINCH in two frames a second
  // apart, the way a full-screen TUI (Claude's, a multi-line prompt) does.
  for (const tile of await page.locator('[data-tile-id]').all()) {
    await tile.locator('.xterm').click();
    await page.keyboard.type("TRAPWINCH() { clear; seq 1 3; sleep 1; seq 1 40 }; clear\n");
  }
  await expect.poll(() => words(page), { timeout: 45_000 }).toEqual(TITLES.map(() => 'Idle'));
  await page.waitForTimeout(3_000);

  const seen = new Set<string>();
  const sample = async () => {
    for (const w of await words(page)) seen.add(w);
    for (const s of await stored()) seen.add(s);
  };
  // Tabs ⇄ tiled ⇄ free layout and a window resize: every pane changes
  // size, so every shell gets a SIGWINCH and redraws its prompt.
  const views = ['Tabbed view', 'Tiled view', 'Tabbed view', 'Tiled view'];
  for (const v of views) {
    await page.locator(`button[aria-label="${v}"]`).click();
    await page.waitForTimeout(1_200);
    await sample();
  }
  for (const [w, h] of [[1000, 700], [1440, 900]] as const) {
    await page.setViewportSize({ width: w, height: h });
    await page.waitForTimeout(1_200);
    await sample();
  }
  // The status task ticks every 2 s and a flip lasts ≥ 5 s: watch long enough.
  for (let i = 0; i < 24; i++) {
    await page.waitForTimeout(500);
    await sample();
  }
  expect([...seen].filter((w) => /Working|Done|working/.test(w))).toEqual([]);
  expect(await words(page)).toEqual(TITLES.map(() => 'Idle'));
});
