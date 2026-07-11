import { test, expect, type APIRequestContext } from '@playwright/test';
import { apiCtx, seedWorkspace } from './seed';

// Desktop BROWSER: Agents-tab session retention. Sessions listed under the
// sidebar "Agents" group must persist — surviving process exit + page reloads —
// until they are explicitly archived or deleted (no auto-removal, no
// age/status filter anywhere in the list path). Mirrors the daemon-side
// guarantee (foreground sessions are exempt from the existence-check pruner —
// see prune_keeps_foreground_sessions_and_prunes_background_ones in
// crates/otto-sessions/src/manager.rs); here we pin the user-visible contract.
//
// Only meaningful on the desktop-browser project; self-skips elsewhere.

// The first navigation pays Vite's whole-app transform on a cold cache (this
// spec often runs alone, straight after a fresh checkout/build); give the
// single test generous headroom so that compile never eats the assertions'
// budget.
test.setTimeout(120_000);

let ctx: APIRequestContext;
let base: string;
let wsId = '';
const NAMES = ['RetainA', 'RetainB'];
const idByTitle: Record<string, string> = {};

/** The session's ACTIVE sidebar row — archived rows render with an extra
 *  `.archived` class inside the (collapsed by default) Archived section, so
 *  exclude them: "leaves the Agents group" must not be satisfied by a
 *  still-collapsed archive. */
function row(page: import('@playwright/test').Page, title: string) {
  return page.locator('.navigator .nested-item:not(.archived)', { hasText: title });
}

test.beforeEach(async ({ page }, info) => {
  test.skip(info.project.name !== 'desktop-browser', 'desktop-browser project only');
  const c = await apiCtx();
  ctx = c.ctx;
  base = c.base;
  wsId = await seedWorkspace(ctx, base);
  for (const title of NAMES) {
    const r = await ctx.post(`${base}/api/v1/workspaces/${wsId}/sessions`, {
      data: { kind: 'agent', provider: 'shell', title, cwd: '/tmp', meta: { origin: 'e2e' } },
    });
    if (!r.ok()) throw new Error(`seed ${title} → ${r.status()} ${await r.text()}`);
    idByTitle[title] = (await r.json()).id as string;
  }
  await page.addInitScript((id) => localStorage.setItem('otto_workspace', id as string), wsId);
  await page.goto('/#/agents');
  await expect(page.getByText('RetainA').first()).toBeVisible({ timeout: 20_000 });
});

test.afterEach(async () => {
  await ctx?.dispose();
});

test('sessions persist under Agents after exit + reload; only archive/delete remove them', async ({ page }) => {
  // 1. Kill every live PTY (the app-quit hook, root-only) — the sessions
  //    become exited/reconnectable, exactly like a long-idle survivor of a
  //    daemon restart.
  const kill = await ctx.post(`${base}/api/v1/app/kill-sessions`);
  expect(kill.ok()).toBeTruthy();

  // 2. Reload: both sessions MUST still be listed under Agents — an exited
  //    session never falls out of the sidebar on its own.
  await page.reload();
  for (const title of NAMES) {
    await expect(row(page, title)).toBeVisible({ timeout: 20_000 });
  }

  // 3. And the rows survive on the API too (no auto-archive happened).
  for (const title of NAMES) {
    const r = await ctx.get(`${base}/api/v1/sessions/${idByTitle[title]}`);
    expect(r.ok()).toBeTruthy();
    expect(((await r.json()) as { archived: boolean }).archived).toBe(false);
  }

  // 4. Archive one — that (and only that) removes it from the Agents group.
  //    (An archive issued OUTSIDE this UI reaches it on the next list load,
  //    so reload before asserting — the contract is presence, not live-sync.)
  const arch = await ctx.post(`${base}/api/v1/sessions/${idByTitle.RetainA}/archive`);
  expect(arch.ok()).toBeTruthy();
  await page.reload();
  await expect(row(page, 'RetainB')).toBeVisible({ timeout: 20_000 });
  await expect(row(page, 'RetainA')).toBeHidden();

  // 5. Delete the other — gone entirely (SessionRemoved drops the row live).
  const del = await ctx.delete(`${base}/api/v1/sessions/${idByTitle.RetainB}`);
  expect(del.ok()).toBeTruthy();
  await expect(row(page, 'RetainB')).toBeHidden({ timeout: 20_000 });

  // 6. A final reload: neither resurfaces under Agents.
  await page.reload();
  await expect(page.locator('.navigator')).toBeVisible({ timeout: 20_000 });
  await expect(row(page, 'RetainA')).toBeHidden();
  await expect(row(page, 'RetainB')).toBeHidden();
});
